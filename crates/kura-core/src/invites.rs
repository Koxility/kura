//! Invitations by email, and passwords set by an admin.
//!
//! Nobody signs up on their own: an account comes from the first-run setup, from an admin, or
//! from an invite an admin sent. An invite fixes the email address and the role; its link
//! carries a random token of which only the SHA-256 is stored, works once and for
//! [`INVITE_TTL`] seconds.

use std::net::IpAddr;

use kura_domain::{Invite, Role, User};
use kura_store::Redeemed;
use lettre::{AsyncTransport, message::Mailbox};

use crate::{
    Core, CoreError, Result,
    auth::{check_email, check_name, check_password, hash, now, random_token, token_hash},
    mail::{Content, address, describe, message, transport},
};

/// How long an invite link works: 7 days.
pub const INVITE_TTL: i64 = 7 * 24 * 60 * 60;

/// Start of the brake's key for an invite link: per address and link, so a person who picks
/// taken names holds up only their own link, and a made-up link holds up nobody else.
const INVITE_BRAKE: &str = "\0invite:";

/// What became of using an invite link.
#[derive(Debug, PartialEq)]
pub enum Registered {
    Created(User),
    NameTaken,
    Invalid,
}

impl Core {
    /// Sends an invite to `email`; whoever opens its link creates an account with `role`.
    /// Waits for the mail server, so the admin sees a failure.
    pub async fn invite(&self, email: &str, role: Role, by: &User) -> Result<Invite> {
        let mail = self.store.mail()?;
        if !mail.enabled() {
            return Err(CoreError::MailNotSetUp);
        }
        let email = email.trim();
        check_email(email)?;
        if self.store.email_in_use(email)? {
            return Err(CoreError::Conflict(
                "an account already uses this email address",
            ));
        }
        let token = random_token()?;
        let invite =
            self.store
                .create_invite(&token_hash(&token), email, role, by.id, now(), INVITE_TTL)?;
        let link = format!("{}/register#token={token}", mail.public_url);
        let days = INVITE_TTL / (24 * 60 * 60);
        let content = Content {
            subject: "Kura: you are invited".into(),
            preview: format!("{} invited you to Kura.", by.name),
            heading: "You are invited".into(),
            paragraphs: vec![format!(
                "{} invited you to Kura, a downloader for series and anime. Use the button below to choose a name and a password for your account.",
                by.name
            )],
            emphasis: Some(format!("The link works once and for the next {days} days.")),
            button: Some(("Create account".into(), link)),
            after: vec![
                "Did not expect this? Ignore this mail; nothing is created without you.".into(),
            ],
        };
        let to = Mailbox::new(None, address(email)?);
        let sent = transport(&mail)?
            .send(message(&mail, to, &content)?)
            .await
            .map_err(|err| CoreError::Mail(describe(&err)));
        if let Err(err) = sent {
            // A link nobody received should not stay valid.
            self.store.delete_invite(invite.id)?;
            return Err(err);
        }
        Ok(invite)
    }

    /// The invites that still work; for admins.
    pub fn invites(&self) -> Result<Vec<Invite>> {
        Ok(self.store.invites(now())?)
    }

    pub fn revoke_invite(&self, id: i64) -> Result<bool> {
        Ok(self.store.delete_invite(id)?)
    }

    /// What an invite link stands for, before someone uses it. `None` for an unknown, used or
    /// expired link. Counts against a brake per address and link, like reset requests.
    pub fn check_invite(&self, token: &str, from: IpAddr) -> Result<Option<Invite>> {
        if !plausible(token) {
            return Ok(None);
        }
        let hashed = token_hash(token.trim());
        self.count_invite_try(from, &hashed)?;
        Ok(self.store.invite_by_token(&hashed, now())?)
    }

    fn count_invite_try(&self, from: IpAddr, hashed: &str) -> Result<()> {
        self.count_reset_request(from, format!("{INVITE_BRAKE}{hashed}"))
    }

    /// Creates the account an invite link stands for. Email and role come from the invite.
    pub async fn register(
        &self,
        token: &str,
        name: &str,
        password: &str,
        from: IpAddr,
    ) -> Result<Registered> {
        let name = check_name(name)?;
        check_password(password)?;
        if !plausible(token) {
            return Ok(Registered::Invalid);
        }
        let token = token_hash(token.trim());
        self.count_invite_try(from, &token)?;
        // Looked up before the password is hashed, so made-up links cost no hashing.
        if self.store.invite_by_token(&token, now())?.is_none() {
            return Ok(Registered::Invalid);
        }
        let hashed = hash(password).await?;
        Ok(
            match self.store.redeem_invite(&token, name, &hashed, now())? {
                Redeemed::Created(user) => Registered::Created(user),
                Redeemed::NameTaken => Registered::NameTaken,
                Redeemed::Invalid => Registered::Invalid,
            },
        )
    }

    /// An admin sets another account's password: its sessions, API keys and reset links end.
    /// `false` for an unknown user.
    pub async fn set_user_password(&self, id: i64, password: &str) -> Result<bool> {
        check_password(password)?;
        if !self.store.users()?.iter().any(|user| user.id == id) {
            return Ok(false);
        }
        let hashed = hash(password).await?;
        self.store.set_password(id, &hashed, "")?;
        Ok(true)
    }
}

/// 64 hex digits, as every link token Kura makes.
fn plausible(token: &str) -> bool {
    let token = token.trim();
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use kura_store::Store;

    use super::*;

    /// Audit finding: tries at invite links counted per address alone and hashed the password
    /// before looking at the link. Now a made-up link is refused at once and holds up only
    /// itself, and these tries never touch the login brake.
    #[tokio::test]
    async fn made_up_links_hold_up_only_themselves() {
        let core = Core::with_defaults(Store::open_in_memory().unwrap()).unwrap();
        let from: IpAddr = "192.0.2.1".parse().unwrap();
        let made_up = "a".repeat(64);
        for _ in 0..5 {
            let tried = core.register(&made_up, "sam", "correct horse", from).await;
            assert_eq!(tried.unwrap(), Registered::Invalid);
        }
        let braked = core.register(&made_up, "sam", "correct horse", from).await;
        assert!(matches!(braked, Err(CoreError::TooManyResets(_))));
        let other = "b".repeat(64);
        let other = core.register(&other, "sam", "correct horse", from).await;
        assert_eq!(other.unwrap(), Registered::Invalid);
        assert!(core.check_invite(&"c".repeat(64), from).unwrap().is_none());
        assert!(core.logins.lock().unwrap().is_empty());
    }
}
