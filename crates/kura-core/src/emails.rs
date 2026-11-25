//! A new email address for the own account counts once the link mailed to it was opened:
//! reset links go to the email, so whoever sets one must show they read mail there. Every
//! address belongs to one account, compared without regard to case.
//!
//! The link carries a random token of which only the SHA-256 is stored; it works once and for
//! [`EMAIL_TTL`] seconds. Admins set addresses at once (see `set_user_email`), and an invite
//! brings its own address, which its link already proved.

use std::net::IpAddr;

use kura_domain::User;
use kura_store::EmailConfirmed;
use lettre::{AsyncTransport, message::Mailbox};

use crate::{
    Core, CoreError, Result,
    auth::{EMAIL_TAKEN, check_email, now, random_token, token_hash},
    mail::{Content, address, describe, message, transport},
};

/// How long a confirmation link works: 24 hours.
pub const EMAIL_TTL: i64 = 24 * 60 * 60;

/// Start of the brake's key for a confirmation link: per address and link.
const EMAIL_BRAKE: &str = "\0email:";

/// What changing the own email did.
#[derive(Debug, PartialEq)]
pub enum EmailChange {
    /// The account has no email now.
    Cleared,
    /// The same address as before (perhaps in other letters' case); kept at once.
    Kept,
    /// A link went to the new address; it counts once that is opened.
    Sent(String),
}

impl Core {
    /// Changes the own email. An empty one clears it at once; the account's own address in
    /// other case is just rewritten. Anything else waits for its link, which this mails and
    /// waits for, so the person sees a failure. Refused when another account or an open invite
    /// has the address, and while no mail server is set up.
    pub async fn change_email(&self, user: &User, email: &str) -> Result<EmailChange> {
        let email = email.trim();
        if email.is_empty() {
            self.set_email_now(user.id, "")?;
            return Ok(EmailChange::Cleared);
        }
        check_email(email)?;
        let current = self
            .store
            .account(user.id)?
            .and_then(|account| account.email);
        if current.is_some_and(|current| current.to_lowercase() == email.to_lowercase()) {
            self.set_email_now(user.id, email)?;
            return Ok(EmailChange::Kept);
        }
        let mail = self.store.mail()?;
        if !mail.enabled() {
            return Err(CoreError::MailNotSetUp);
        }
        let token = random_token()?;
        let started =
            self.store
                .start_email_change(user.id, email, &token_hash(&token), now(), EMAIL_TTL)?;
        if !started {
            return Err(EMAIL_TAKEN);
        }
        let link = format!("{}/confirm-email#token={token}", mail.public_url);
        let hours = EMAIL_TTL / (60 * 60);
        let content = Content {
            subject: "Kura: confirm your email address".into(),
            preview: "One click to send Kura's password reset links here.".into(),
            heading: format!("Hi {},", user.name),
            paragraphs: vec![format!(
                "Someone asked Kura to send the password reset links of the account {} to this address. Use the button below to confirm that it is yours.",
                user.name
            )],
            emphasis: Some(format!(
                "The link works once and for the next {hours} hours."
            )),
            button: Some(("Confirm email".into(), link)),
            after: vec![
                "Did not ask for this? Ignore this mail; nothing changes without the link.".into(),
            ],
        };
        let to = Mailbox::new(Some(user.name.clone()), address(email)?);
        let sent = transport(&mail)?
            .send(message(&mail, to, &content)?)
            .await
            .map_err(|err| CoreError::Mail(describe(&err)));
        if let Err(err) = sent {
            // A link nobody received should not stay valid.
            self.store.cancel_email_change(user.id)?;
            return Err(err);
        }
        Ok(EmailChange::Sent(email.to_owned()))
    }

    /// Opens a confirmation link: its address becomes the account's email. `false` for an
    /// unknown, used or expired link; refused when another account or an invite has taken the
    /// address meanwhile. Counts against a brake per address and link.
    pub fn confirm_email(&self, token: &str, from: IpAddr) -> Result<bool> {
        let token = token.trim();
        let plausible = token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !plausible {
            return Ok(false);
        }
        let hashed = token_hash(token);
        self.count_reset_request(from, format!("{EMAIL_BRAKE}{hashed}"))?;
        match self.store.confirm_email_change(&hashed, now())? {
            EmailConfirmed::Changed { .. } => Ok(true),
            EmailConfirmed::Taken => Err(EMAIL_TAKEN),
            EmailConfirmed::Invalid => Ok(false),
        }
    }
}
