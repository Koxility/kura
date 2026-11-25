//! Local accounts and login sessions.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr},
    sync::PoisonError,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use kura_domain::{Account, ApiKey, Role, Theme, User};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::{Core, CoreError, Result};

/// How long a login lasts, in seconds.
pub const SESSION_TTL: i64 = 30 * 24 * 60 * 60;

/// How long after logging in the password counts as just typed (seconds).
pub const RECENT_LOGIN: i64 = 10 * 60;

/// Passwords hashed or checked at the same time. Argon2 takes about 19 MiB each time, and
/// logging in needs no account, so without a limit a burst of logins takes gigabytes.
static HASHING: Semaphore = Semaphore::const_new(4);

/// Wrong logins for one name from one address before a pause is needed.
const FREE_TRIES: u32 = 5;

/// The pause after the last free try; every further try doubles it, up to [`LONGEST_PAUSE`].
const FIRST_PAUSE: Duration = Duration::from_secs(30);
const LONGEST_PAUSE: Duration = Duration::from_secs(15 * 60);

/// Wrong logins are forgotten this long after the last one.
const FORGET_AFTER: Duration = Duration::from_secs(15 * 60);

/// More than any honest use needs; keeps the list small when someone tries many names.
const MOST_TRACKED: usize = 10_000;

/// Logins tried for one name from one address since the last one that worked.
#[derive(Debug)]
pub(crate) struct Tries {
    count: u32,
    last: Instant,
    /// No further try before this.
    paused_until: Option<Instant>,
}

/// By address and lower-case name. The address is the one the connection comes from: behind
/// a reverse proxy that is the proxy, and the limit is per name.
pub(crate) type LoginTries = HashMap<(IpAddr, String), Tries>;

impl Core {
    /// Whether this is a fresh installation without any account.
    pub fn needs_setup(&self) -> Result<bool> {
        Ok(!self.store.has_users()?)
    }

    /// Creates the first admin. Returns `None` when the installation already has accounts.
    pub async fn setup(&self, name: &str, password: &str) -> Result<Option<User>> {
        let name = check(name, password)?;
        let hash = hash(password).await?;
        Ok(self.store.create_first_admin(name, &hash)?)
    }

    /// Returns `None` when the name is taken.
    pub async fn create_user(
        &self,
        name: &str,
        password: &str,
        role: Role,
    ) -> Result<Option<User>> {
        let name = check(name, password)?;
        let hash = hash(password).await?;
        Ok(self.store.create_user(name, &hash, role)?)
    }

    pub fn users(&self) -> Result<Vec<User>> {
        Ok(self.store.users()?)
    }

    /// Returns whether a user was deleted. The last admin cannot be deleted.
    pub fn delete_user(&self, id: i64) -> Result<bool> {
        Ok(self.store.delete_user(id)?)
    }

    /// Checks name and password and starts a session. Returns the user and the session token,
    /// or `None` when either is wrong. `from` is the address the login comes from: after a few
    /// wrong tries for a name it has to wait, so passwords cannot be guessed at speed.
    pub async fn login(
        &self,
        name: &str,
        password: &str,
        from: IpAddr,
    ) -> Result<Option<(User, String)>> {
        let name = name.trim();
        // No account has a longer name; this also keeps long names out of the list of tries.
        if name.chars().count() > 64 {
            return Ok(None);
        }
        let key = (network(from), name.to_lowercase());
        // Counted before the check, so tries sent at the same time count too.
        self.count_try(&key)?;

        let Some((user, stored)) = self.store.credentials(name)? else {
            // Spend the same time as for a wrong password, so names cannot be probed.
            hash(password).await?;
            return Ok(None);
        };
        if !verify(password, stored).await? {
            return Ok(None);
        }
        self.login_tries().remove(&key);

        let token = random_token()?;
        let hashed = token_hash(&token);
        self.store
            .create_session(&hashed, user.id, now(), SESSION_TTL)?;
        Ok(Some((user, token)))
    }

    /// Counts one login try, or refuses it while a pause is running.
    pub(crate) fn count_try(&self, key: &(IpAddr, String)) -> Result<()> {
        count_in(&mut self.login_tries(), key)
    }

    /// Counts one try at something a mailed link opens, in a list of its own; refused like a
    /// login while a pause is running.
    pub(crate) fn count_link(&self, key: &(IpAddr, String)) -> Result<()> {
        count_in(
            &mut self.links.lock().unwrap_or_else(PoisonError::into_inner),
            key,
        )
    }

    fn login_tries(&self) -> std::sync::MutexGuard<'_, LoginTries> {
        self.logins.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The user a session token belongs to, while the session is valid.
    pub fn session(&self, token: &str) -> Result<Option<User>> {
        Ok(self.store.session_user(&token_hash(token), now())?)
    }

    /// Changes `user`'s password after checking the current one. Other sessions of the user
    /// end; the one in `token` stays. Returns `false` when the current password is wrong.
    pub async fn change_password(
        &self,
        user: &User,
        token: &str,
        current: &str,
        new: &str,
    ) -> Result<bool> {
        check(&user.name, new)?;
        let Some((_, stored)) = self.store.credentials(&user.name)? else {
            return Ok(false);
        };
        if !verify(current, stored).await? {
            return Ok(false);
        }
        let hashed = hash(new).await?;
        self.store
            .set_password(user.id, &hashed, &token_hash(token))?;
        Ok(true)
    }

    /// Whether the session began at most `RECENT_LOGIN` ago: right after typing the password,
    /// that counts as having typed it again.
    pub fn recent_login(&self, token: &str) -> Result<bool> {
        let expiry = self.store.session_expiry(&token_hash(token))?;
        Ok(expiry.is_some_and(|expires| now() - (expires - SESSION_TTL) <= RECENT_LOGIN))
    }

    /// The account said "not now" to adding an email address.
    pub fn skip_email(&self, user: &User) -> Result<()> {
        Ok(self.store.mark_email_asked(user.id)?)
    }

    /// Whether `password` is the user's current one.
    pub async fn check_own_password(&self, user: &User, password: &str) -> Result<bool> {
        let Some((_, stored)) = self.store.credentials(&user.name)? else {
            return Ok(false);
        };
        verify(password, stored).await
    }

    pub fn logout(&self, token: &str) -> Result<()> {
        Ok(self.store.delete_session(&token_hash(token))?)
    }

    // The own account

    /// The own account. The web app asks for an email only while one could be used, that is
    /// while a mail server is set up.
    pub fn account(&self, user: &User) -> Result<Account> {
        let account = self.store.account(user.id)?;
        let mut account = account.ok_or(CoreError::Invalid("this account no longer exists"))?;
        account.ask_email &= self.store.mail()?.enabled();
        Ok(account)
    }

    /// Returns `false` when another user has the name.
    pub fn rename(&self, user: &User, name: &str) -> Result<bool> {
        let name = check_name(name)?;
        Ok(self.store.rename_user(user.id, name)?)
    }

    /// An admin sets the email of any user, at once and without a confirmation mail: the
    /// admin answers for the address. `false` when there is no such user; refused when another
    /// account or an open invite has the address.
    pub fn set_user_email(&self, id: i64, email: &str) -> Result<bool> {
        if !self.store.users()?.iter().any(|user| user.id == id) {
            return Ok(false);
        }
        self.set_email_now(id, email)?;
        Ok(true)
    }

    /// Refuses an address that is not one, or that another account or an open invite has.
    pub fn check_email_free(&self, email: &str) -> Result<()> {
        let email = email.trim();
        check_email(email)?;
        if self.store.email_taken(email, None, now())? {
            return Err(EMAIL_TAKEN);
        }
        Ok(())
    }

    /// Users' email entries; for admins.
    pub fn user_entries(&self) -> Result<Vec<kura_domain::UserEntry>> {
        Ok(self.store.user_entries()?)
    }

    /// Sets an email, or clears it for an empty one, without asking the address first.
    pub(crate) fn set_email_now(&self, id: i64, email: &str) -> Result<()> {
        let email = email.trim();
        if !email.is_empty() {
            check_email(email)?;
        }
        let email = Some(email).filter(|email| !email.is_empty());
        if self.store.set_email(id, email, now())? {
            Ok(())
        } else {
            Err(EMAIL_TAKEN)
        }
    }

    pub fn set_theme(&self, user: &User, theme: &Theme) -> Result<()> {
        // Piece names pick a module in the web app, so they stay plain ids.
        let plain = |piece: &String| {
            !piece.is_empty()
                && piece.len() <= 64
                && piece
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        };
        let library = &theme.ascii_library;
        if library.len() > MOST_ASCII_PIECES || !library.iter().all(plain) {
            return Err(CoreError::Invalid("no such ASCII scene"));
        }
        if !library.contains(&theme.ascii_piece) {
            return Err(CoreError::Invalid("pick an ASCII scene from your list"));
        }
        if theme.volume > 100 {
            return Err(CoreError::Invalid("the volume goes from 0 to 100"));
        }
        if let Some(song) = &theme.song
            && !self.songs()?.iter().any(|known| known.id == *song)
        {
            return Err(CoreError::Invalid("no such song"));
        }
        Ok(self.store.set_theme(user.id, theme)?)
    }

    // API keys, for programs such as Musubi

    /// Creates a key and returns it with the secret, which is shown this once.
    pub fn create_api_key(&self, user: &User, name: &str) -> Result<(ApiKey, String)> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(CoreError::Invalid(
                "the key needs a name of 1 to 64 characters",
            ));
        }
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|err| CoreError::Hashing(err.to_string()))?;
        let secret = format!("{API_KEY_PREFIX}{}", hex(&secret));
        let key = self
            .store
            .create_api_key(user.id, name, &token_hash(&secret), now())?;
        Ok((key, secret))
    }

    pub fn api_keys(&self, user: &User) -> Result<Vec<ApiKey>> {
        Ok(self.store.api_keys(user.id)?)
    }

    /// Returns whether one of the user's keys was deleted.
    pub fn delete_api_key(&self, user: &User, id: i64) -> Result<bool> {
        Ok(self.store.delete_api_key(id, user.id)?)
    }

    /// The user an API key belongs to.
    pub fn api_key_user(&self, secret: &str) -> Result<Option<User>> {
        if !secret.starts_with(API_KEY_PREFIX) {
            return Ok(None);
        }
        Ok(self.store.api_key_user(&token_hash(secret), now())?)
    }
}

/// Counts one try for `key` in `tries`, or refuses it while a pause is running.
fn count_in(tries: &mut LoginTries, key: &(IpAddr, String)) -> Result<()> {
    let now = Instant::now();
    if tries.len() >= MOST_TRACKED && !tries.contains_key(key) {
        tries.retain(|_, tried| !tried.forgotten(now));
        if tries.len() >= MOST_TRACKED {
            tries.clear();
        }
    }
    let tried = tries.entry(key.clone()).or_insert(Tries {
        count: 0,
        last: now,
        paused_until: None,
    });
    if tried.forgotten(now) {
        tried.count = 0;
    }
    if let Some(until) = tried.paused_until.filter(|until| *until > now) {
        let seconds = until.duration_since(now).as_secs().max(1);
        return Err(CoreError::TooManyLogins(seconds));
    }
    tried.count += 1;
    tried.last = now;
    tried.paused_until = pause_after(tried.count).map(|pause| now + pause);
    Ok(())
}

/// Every account has its own email address, and an invited address is kept for its invite.
pub(crate) const EMAIL_TAKEN: CoreError =
    CoreError::Conflict("another account already uses this email address, or was invited with it");

/// More scenes than anyone keeps; bounds what one account stores.
const MOST_ASCII_PIECES: usize = 100;

/// Every API key starts with this, so one is told apart from a session token and found when
/// it leaks into a log or a repository.
pub const API_KEY_PREFIX: &str = "kura_";

impl Tries {
    fn forgotten(&self, now: Instant) -> bool {
        let paused = self.paused_until.is_some_and(|until| until > now);
        !paused && now.duration_since(self.last) >= FORGET_AFTER
    }
}

/// The wait after `count` tries without a login that worked; `None` while tries are free.
fn pause_after(count: u32) -> Option<Duration> {
    let extra = count.checked_sub(FREE_TRIES)?;
    let doubled = FIRST_PAUSE.checked_mul(2u32.checked_pow(extra).unwrap_or(u32::MAX));
    Some(doubled.map_or(LONGEST_PAUSE, |pause| pause.min(LONGEST_PAUSE)))
}

/// The address that counts for the limit. One customer usually holds a whole IPv6 `/64`, so
/// its addresses count as one.
pub(crate) fn network(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !u128::from(u64::MAX))),
        },
        v4 => v4,
    }
}

/// Returns the trimmed name.
pub(crate) fn check_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(CoreError::Invalid("the name must have 1 to 64 characters"));
    }
    Ok(name)
}

/// Returns the trimmed name.
fn check<'a>(name: &'a str, password: &str) -> Result<&'a str> {
    let name = check_name(name)?;
    check_password(password)?;
    Ok(name)
}

/// An address a reset mail can go to: one line, `name@domain.tld`, as a mail server takes it.
pub fn check_email(email: &str) -> Result<()> {
    let email = email.trim();
    let valid = email.len() <= 254
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
        })
        && !email.contains(char::is_whitespace)
        && crate::mail::address(email).is_ok();
    if valid {
        Ok(())
    } else {
        Err(CoreError::Invalid(
            "this does not look like an email address",
        ))
    }
}

pub(crate) fn check_password(password: &str) -> Result<()> {
    // The upper limit keeps hashing cheap for anything a client sends.
    if !(8..=256).contains(&password.chars().count()) {
        return Err(CoreError::Invalid(
            "the password must have 8 to 256 characters",
        ));
    }
    Ok(())
}

/// 32 random bytes from the operating system, as 64 hex digits: for sessions and reset links.
pub(crate) fn random_token() -> Result<String> {
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(|err| CoreError::Hashing(err.to_string()))?;
    Ok(hex(&token))
}

/// Checking takes tens of milliseconds, so it runs off the async threads.
async fn verify(password: &str, stored: String) -> Result<bool> {
    let _turn = hashing_turn().await?;
    let password = password.to_owned();
    let matches = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .verify_password(password.as_bytes(), stored.as_str())
            .is_ok()
    });
    matches
        .await
        .map_err(|err| CoreError::Hashing(err.to_string()))
}

/// Hashing takes tens of milliseconds, so it runs off the async threads.
pub(crate) async fn hash(password: &str) -> Result<String> {
    let _turn = hashing_turn().await?;
    let password = password.to_owned();
    let hashing = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
    });
    match hashing.await {
        Ok(Ok(hash)) => Ok(hash),
        Ok(Err(err)) => Err(CoreError::Hashing(err.to_string())),
        Err(err) => Err(CoreError::Hashing(err.to_string())),
    }
}

/// Waits until fewer than the allowed number of passwords are being hashed.
async fn hashing_turn() -> Result<tokio::sync::SemaphorePermit<'static>> {
    let turn = HASHING.acquire().await;
    turn.map_err(|err| CoreError::Hashing(err.to_string()))
}

/// Only the hash is stored, so a copy of the database holds no usable session.
pub(crate) fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn now() -> i64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH);
    since_epoch.map_or(0, |elapsed| elapsed.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use kura_store::Store;

    use super::*;

    #[test]
    fn only_a_fresh_login_counts_as_the_password() {
        let store = Store::open_in_memory().unwrap();
        let user = store
            .create_user("sam", "hash", Role::User)
            .unwrap()
            .unwrap();
        // One session from just now, one from an hour ago.
        store
            .create_session(&token_hash("fresh"), user.id, now(), SESSION_TTL)
            .unwrap();
        store
            .create_session(&token_hash("old"), user.id, now() - 3600, SESSION_TTL)
            .unwrap();
        let core = Core::with_defaults(store).unwrap();
        assert!(core.recent_login("fresh").unwrap());
        assert!(!core.recent_login("old").unwrap());
        assert!(!core.recent_login("unknown").unwrap());
    }

    const HOME: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 20));

    #[tokio::test]
    async fn accounts_from_setup_to_logout() {
        let core = Core::with_defaults(Store::open_in_memory().unwrap()).unwrap();
        assert!(core.needs_setup().unwrap());

        let short = core.setup("admin", "short").await;
        assert!(matches!(short, Err(CoreError::Invalid(_))));
        let nameless = core.setup("  ", "long enough").await;
        assert!(matches!(nameless, Err(CoreError::Invalid(_))));

        let admin = core
            .setup(" admin ", "correct horse")
            .await
            .unwrap()
            .unwrap();
        assert_eq!((admin.name.as_str(), admin.role), ("admin", Role::Admin));
        assert!(!core.needs_setup().unwrap());
        assert!(
            core.setup("second", "correct horse")
                .await
                .unwrap()
                .is_none()
        );

        assert!(
            core.login("admin", "wrong horse", HOME)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            core.login("nobody", "correct horse", HOME)
                .await
                .unwrap()
                .is_none()
        );
        let (user, token) = core
            .login("Admin", "correct horse", HOME)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user, admin);
        assert_eq!(token.len(), 64);

        assert_eq!(core.session(&token).unwrap(), Some(admin));
        assert_eq!(core.session("not a token").unwrap(), None);
        core.logout(&token).unwrap();
        assert_eq!(core.session(&token).unwrap(), None);

        // Changing the password needs the current one and keeps only this session.
        let (_, other) = core
            .login("admin", "correct horse", HOME)
            .await
            .unwrap()
            .unwrap();
        let (_, here) = core
            .login("admin", "correct horse", HOME)
            .await
            .unwrap()
            .unwrap();
        let wrong = core.change_password(&user, &here, "wrong horse", "new horse 1");
        assert!(!wrong.await.unwrap());
        let short = core
            .change_password(&user, &here, "correct horse", "short")
            .await;
        assert!(matches!(short, Err(CoreError::Invalid(_))));
        let changed = core.change_password(&user, &here, "correct horse", "new horse 1");
        assert!(changed.await.unwrap());
        assert!(core.session(&here).unwrap().is_some());
        assert!(core.session(&other).unwrap().is_none());
        assert!(
            core.login("admin", "correct horse", HOME)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            core.login("admin", "new horse 1", HOME)
                .await
                .unwrap()
                .is_some()
        );

        let taken = core.create_user("ADMIN", "correct horse", Role::User).await;
        assert!(taken.unwrap().is_none());
        let sam = core.create_user("sam", "battery staple", Role::User).await;
        assert_eq!(sam.unwrap().unwrap().role, Role::User);
        assert_eq!(core.users().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn wrong_logins_have_to_wait() {
        let core = Core::with_defaults(Store::open_in_memory().unwrap()).unwrap();
        core.setup("admin", "correct horse").await.unwrap().unwrap();
        let elsewhere: IpAddr = "2001:db8:1:2::7".parse().unwrap();

        for _ in 0..FREE_TRIES {
            let wrong = core.login("admin", "wrong horse", HOME).await;
            assert!(wrong.unwrap().is_none());
        }
        // Not even the right password gets through, however the name is spelled.
        for name in ["admin", " ADMIN "] {
            let refused = core.login(name, "correct horse", HOME).await;
            assert!(matches!(refused, Err(CoreError::TooManyLogins(1..=30))));
        }
        // Another address and another name are not held up.
        let (_, token) = core
            .login("admin", "correct horse", elsewhere)
            .await
            .unwrap()
            .unwrap();
        assert!(core.session(&token).unwrap().is_some());
        assert!(core.login("sam", "wrong horse", HOME).await.is_ok());

        // A login that works starts the count again.
        for _ in 0..FREE_TRIES - 1 {
            let wrong = core.login("admin", "wrong horse", elsewhere).await;
            assert!(wrong.unwrap().is_none());
        }
        let right = core.login("admin", "correct horse", elsewhere).await;
        assert!(right.unwrap().is_some());
        let wrong = core.login("admin", "wrong horse", elsewhere).await;
        assert!(wrong.unwrap().is_none());

        // A name no account can have is refused without being remembered.
        let long = "a".repeat(65);
        assert!(
            core.login(&long, "correct horse", HOME)
                .await
                .unwrap()
                .is_none()
        );
        assert!(!core.login_tries().keys().any(|(_, name)| *name == long));
    }

    #[test]
    fn pauses_double_and_addresses_count_by_network() {
        assert_eq!(pause_after(FREE_TRIES - 1), None);
        assert_eq!(pause_after(FREE_TRIES), Some(FIRST_PAUSE));
        assert_eq!(pause_after(FREE_TRIES + 2), Some(FIRST_PAUSE * 4));
        assert_eq!(pause_after(FREE_TRIES + 20), Some(LONGEST_PAUSE));
        assert_eq!(pause_after(u32::MAX), Some(LONGEST_PAUSE));

        let network_of = |address: &str| network(address.parse().unwrap());
        assert_eq!(
            network_of("2001:db8:1:2::7"),
            network_of("2001:db8:1:2:ffff::1")
        );
        assert_ne!(network_of("2001:db8:1:2::7"), network_of("2001:db8:1:3::7"));
        assert_eq!(network_of("::ffff:192.168.1.20"), HOME);
        assert_ne!(network_of("192.168.1.21"), HOME);
    }
}
