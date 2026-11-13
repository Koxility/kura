//! Local accounts and login sessions.

use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use kura_domain::{Role, User};
use sha2::{Digest, Sha256};

use crate::{Core, CoreError, Result};

/// How long a login lasts, in seconds.
pub const SESSION_TTL: i64 = 30 * 24 * 60 * 60;

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
    /// or `None` when either is wrong.
    pub async fn login(&self, name: &str, password: &str) -> Result<Option<(User, String)>> {
        let Some((user, stored)) = self.store.credentials(name.trim())? else {
            // Spend the same time as for a wrong password, so names cannot be probed.
            hash(password).await?;
            return Ok(None);
        };
        if !verify(password, stored).await? {
            return Ok(None);
        }

        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(|err| CoreError::Hashing(err.to_string()))?;
        let token = hex(&token);
        let hashed = token_hash(&token);
        self.store
            .create_session(&hashed, user.id, now(), SESSION_TTL)?;
        Ok(Some((user, token)))
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

    pub fn logout(&self, token: &str) -> Result<()> {
        Ok(self.store.delete_session(&token_hash(token))?)
    }
}

/// Returns the trimmed name.
fn check<'a>(name: &'a str, password: &str) -> Result<&'a str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(CoreError::Invalid("the name must have 1 to 64 characters"));
    }
    // The upper limit keeps hashing cheap for anything a client sends.
    if !(8..=256).contains(&password.chars().count()) {
        return Err(CoreError::Invalid(
            "the password must have 8 to 256 characters",
        ));
    }
    Ok(name)
}

/// Checking takes tens of milliseconds, so it runs off the async threads.
async fn verify(password: &str, stored: String) -> Result<bool> {
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
async fn hash(password: &str) -> Result<String> {
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

/// Only the hash is stored, so a copy of the database holds no usable session.
fn token_hash(token: &str) -> String {
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

        assert!(core.login("admin", "wrong horse").await.unwrap().is_none());
        assert!(
            core.login("nobody", "correct horse")
                .await
                .unwrap()
                .is_none()
        );
        let (user, token) = core.login("Admin", "correct horse").await.unwrap().unwrap();
        assert_eq!(user, admin);
        assert_eq!(token.len(), 64);

        assert_eq!(core.session(&token).unwrap(), Some(admin));
        assert_eq!(core.session("not a token").unwrap(), None);
        core.logout(&token).unwrap();
        assert_eq!(core.session(&token).unwrap(), None);

        // Changing the password needs the current one and keeps only this session.
        let (_, other) = core.login("admin", "correct horse").await.unwrap().unwrap();
        let (_, here) = core.login("admin", "correct horse").await.unwrap().unwrap();
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
            core.login("admin", "correct horse")
                .await
                .unwrap()
                .is_none()
        );
        assert!(core.login("admin", "new horse 1").await.unwrap().is_some());

        let taken = core.create_user("ADMIN", "correct horse", Role::User).await;
        assert!(taken.unwrap().is_none());
        let sam = core.create_user("sam", "battery staple", Role::User).await;
        assert_eq!(sam.unwrap().unwrap().role, Role::User);
        assert_eq!(core.users().unwrap().len(), 2);
    }
}
