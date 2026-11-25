//! Mails Kura sends: password reset links, and a test mail to check the mail server.
//!
//! The admin sets the mail server (`Mail`). A reset link carries a random token; only its
//! SHA-256 is stored, it works once and for [`RESET_TTL`] seconds. Whether an account exists
//! is never told: the answer to a reset request is the same for everyone, and the mail goes
//! out after the answer.

use std::{net::IpAddr, str::FromStr, sync::Arc, time::Duration};

use kura_domain::{Mail, MailEncryption, UserEntry};
use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
    transport::smtp::authentication::Credentials,
};

use crate::{
    Core, CoreError, Result,
    auth::{check_password, hash, now, random_token, token_hash},
};

/// How long a reset link works: 30 minutes.
pub const RESET_TTL: i64 = 30 * 60;

/// A new reset link for the same account at most this often (seconds); asking again sooner
/// sends nothing, so nobody can flood someone's inbox.
const RESET_EVERY: i64 = 2 * 60;

/// How long Kura waits for the mail server.
const MAIL_TIMEOUT: Duration = Duration::from_secs(20);

/// The longest name or address a reset is asked for.
const MOST_NAME: usize = 254;

/// Start of the brake's key for reset requests: per address and the name or email asked
/// for, so one person asking too often holds up only their own requests.
const RESET_BRAKE: &str = "\0password-reset:";

const LAYOUT: &str = include_str!("mail/layout.html");

impl Core {
    /// The mail server settings, with the password.
    pub fn mail(&self) -> Result<Mail> {
        Ok(self.store.mail()?)
    }

    /// Saves the mail server. An empty password keeps the stored one, so the browser never
    /// needs to know it; an empty host switches mail off and forgets the password too.
    pub fn save_mail(&self, mail: &Mail) -> Result<Mail> {
        let mut mail = tidy(mail.clone());
        if mail.host.is_empty() {
            mail = Mail::default();
        } else if mail.password.is_empty() {
            // Only for the very same server and account: otherwise the stored password would
            // go to whatever server is entered, and an admin could read it that way.
            let stored = self.store.mail()?;
            let same = stored.host.eq_ignore_ascii_case(&mail.host)
                && stored.port == mail.port
                && stored.encryption == mail.encryption
                && stored.username == mail.username;
            if same && !stored.username.is_empty() {
                mail.password = stored.password;
            }
        }
        check_mail(&mail)?;
        self.store.save_mail(&mail)?;
        Ok(mail)
    }

    /// Sends a test mail to `to` with the saved settings, and waits for the mail server.
    pub async fn send_test_mail(&self, to: &str) -> Result<()> {
        let mail = self.store.mail()?;
        if !mail.enabled() {
            return Err(CoreError::MailNotSetUp);
        }
        let to = Mailbox::new(None, address(to)?);
        let content = Content {
            subject: "Kura: test mail".into(),
            preview: "Kura can send mails.".into(),
            heading: "Mail works".into(),
            paragraphs: vec![
                "This is a test mail from Kura. Password reset links and invitations are sent the same way.".into(),
            ],
            emphasis: None,
            button: Some(("Open Kura".into(), mail.public_url.clone())),
            after: vec![],
        };
        let message = message(&mail, to, &content)?;
        transport(&mail)?
            .send(message)
            .await
            .map_err(|err| CoreError::Mail(describe(&err)))?;
        Ok(())
    }

    /// Whether password reset by mail is set up.
    pub fn password_reset_enabled(&self) -> Result<bool> {
        Ok(self.store.mail()?.enabled())
    }

    /// Someone asks for a reset link for a name or an email address. Answers the same way,
    /// and after the same work, whether or not such an account exists: looking the account up,
    /// making the link and sending the mail all happen afterwards, in the background.
    pub fn request_password_reset(
        self: &Arc<Self>,
        name_or_email: &str,
        from: IpAddr,
    ) -> Result<()> {
        if !self.store.mail()?.enabled() {
            return Err(CoreError::MailNotSetUp);
        }
        let wanted = name_or_email.trim().to_owned();
        // No account has such a name or address; this also keeps them out of the brake's list.
        if wanted.is_empty() || wanted.chars().count() > MOST_NAME {
            return Ok(());
        }
        let target = format!("{RESET_BRAKE}{}", wanted.to_lowercase());
        self.count_reset_request(from, target)?;
        let core = self.clone();
        tokio::spawn(async move {
            if let Err(err) = core.send_reset_links(&wanted) {
                tracing::warn!("a password reset could not be prepared: {err}");
            }
        });
        Ok(())
    }

    /// Makes a link for each account `wanted` names that has an email, and sends it.
    fn send_reset_links(&self, wanted: &str) -> Result<()> {
        let mail = self.store.mail()?;
        let now = now();
        for user in self.store.users_named(wanted)? {
            let Some(email) = user.email.clone() else {
                continue;
            };
            let recent = self.store.last_reset(user.id, now)?;
            if recent.is_some_and(|made| now - made < RESET_EVERY) {
                continue;
            }
            let token = random_token()?;
            self.store
                .create_reset(&token_hash(&token), user.id, now, RESET_TTL)?;
            let link = format!("{}/reset-password#token={token}", mail.public_url);
            let mail = mail.clone();
            tokio::spawn(async move {
                if let Err(err) = send_reset(&mail, &user, &email, &link).await {
                    tracing::warn!("the password reset mail for {} failed: {err}", user.name);
                }
            });
        }
        Ok(())
    }

    /// Sets a new password with a reset link's token. The link works once; all sessions, API
    /// keys and other links of the account end. `false` for an unknown, used or expired link.
    pub async fn reset_password(&self, token: &str, password: &str) -> Result<bool> {
        // Checked first, so a password that is too short does not use the link up.
        check_password(password)?;
        let token = token.trim();
        let plausible = token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !plausible {
            return Ok(false);
        }
        let Some(user_id) = self.store.take_reset(&token_hash(token), now())? else {
            return Ok(false);
        };
        let hashed = hash(password).await?;
        // No session is kept: whoever reset the password logs in afresh.
        self.store.set_password(user_id, &hashed, "")?;
        Ok(true)
    }

    /// The brake for everything a mailed link opens, per address and `target` (what is asked
    /// for: a name, an email, a link's token), like the one for logins per address and name.
    pub(crate) fn count_reset_request(&self, from: IpAddr, target: String) -> Result<()> {
        self.count_link(&(crate::auth::network(from), target))
            .map_err(|err| match err {
                CoreError::TooManyLogins(seconds) => CoreError::TooManyResets(seconds),
                other => other,
            })
    }
}

/// Trims what people paste around values.
fn tidy(mut mail: Mail) -> Mail {
    mail.host = mail.host.trim().to_owned();
    mail.username = mail.username.trim().to_owned();
    mail.from = mail.from.trim().to_owned();
    mail.public_url = mail.public_url.trim().trim_end_matches('/').to_owned();
    mail
}

/// Refuses settings that could not work or that would put something else into a mail.
pub(crate) fn check_mail(mail: &Mail) -> Result<()> {
    let invalid = |message| Err(CoreError::Invalid(message));
    if mail.host.is_empty() {
        return Ok(());
    }
    let host_ok = mail.host.len() <= 253
        && mail
            .host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !host_ok {
        return invalid("the mail server is a name like smtp.example.org, without http://");
    }
    if mail.port == 0 {
        return invalid("the mail server needs a port, e.g. 587");
    }
    let lines = |text: &str| text.contains(['\r', '\n']);
    if [&mail.username, &mail.password, &mail.from, &mail.public_url]
        .iter()
        .any(|text| lines(text))
    {
        return invalid("the mail settings cannot contain line breaks");
    }
    if mail.username.len() > 512 || mail.password.len() > 512 {
        return invalid("the user name or password is too long");
    }
    if mail.encryption == MailEncryption::None && !mail.username.is_empty() {
        return invalid("a mail password is only sent over an encrypted connection");
    }
    if Mailbox::from_str(&mail.from).is_err() {
        return invalid("the sender looks like noreply@example.org or Kura <noreply@example.org>");
    }
    let url = reqwest::Url::parse(&mail.public_url);
    let url_ok = url.is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });
    if !url_ok {
        return invalid("Kura's address looks like https://kura.example.org");
    }
    Ok(())
}

/// An email address as a mail server takes it.
pub(crate) fn address(text: &str) -> Result<Address> {
    let text = text.trim();
    if text.contains(['\r', '\n']) {
        return Err(CoreError::Invalid(
            "this does not look like an email address",
        ));
    }
    Address::from_str(text)
        .map_err(|_| CoreError::Invalid("this does not look like an email address"))
}

pub(crate) async fn send_reset(
    mail: &Mail,
    user: &UserEntry,
    email: &str,
    link: &str,
) -> Result<()> {
    let to = Mailbox::new(Some(user.name.clone()), address(email)?);
    let minutes = RESET_TTL / 60;
    let content = Content {
        subject: "Kura: reset your password".into(),
        preview: "A link to choose a new password for Kura.".into(),
        heading: format!("Hi {},", user.name),
        paragraphs: vec![
            "Someone asked to reset the password of your Kura account. Use the button below to choose a new one.".into(),
        ],
        emphasis: Some(format!("The link works once and for the next {minutes} minutes.")),
        button: Some(("Reset password".into(), link.to_owned())),
        after: vec![
            "Afterwards you are logged out everywhere, and the account's API keys are deleted.".into(),
            "Did not ask for this? Ignore this mail; your password stays as it is.".into(),
        ],
    };
    let message = message(mail, to, &content)?;
    transport(mail)?
        .send(message)
        .await
        .map_err(|err| CoreError::Mail(describe(&err)))?;
    Ok(())
}

/// What a mail says, as plain text; the layout and escaping are added when it is built.
pub(crate) struct Content {
    pub subject: String,
    pub preview: String,
    /// The greeting at the top, e.g. "Hi sam,".
    pub heading: String,
    pub paragraphs: Vec<String>,
    /// Bold, at the end of the last paragraph: what matters most, e.g. how long a link works.
    pub emphasis: Option<String>,
    /// Text and address of the one button; the address is also written out below it.
    pub button: Option<(String, String)>,
    /// Paragraphs below the button.
    pub after: Vec<String>,
}

pub(crate) fn message(mail: &Mail, to: Mailbox, content: &Content) -> Result<Message> {
    let from = Mailbox::from_str(&mail.from)
        .map_err(|_| CoreError::Invalid("the sender of the mail settings is not an address"))?;
    Message::builder()
        .from(from)
        .to(to)
        .subject(&content.subject)
        .multipart(MultiPart::alternative_plain_html(
            plain(content),
            html(content),
        ))
        .map_err(|err| CoreError::Mail(err.to_string()))
}

/// The plain-text part, for mail programs that do not show HTML. Same order as the HTML.
pub(crate) fn plain(content: &Content) -> String {
    let mut text = format!("Kura\n\n{}\n\n", content.heading);
    let last = content.paragraphs.len().saturating_sub(1);
    for (index, paragraph) in content.paragraphs.iter().enumerate() {
        text.push_str(paragraph);
        if index == last
            && let Some(emphasis) = &content.emphasis
        {
            text.push(' ');
            text.push_str(emphasis);
        }
        text.push_str("\n\n");
    }
    if let Some((label, url)) = &content.button {
        text.push_str(&format!("{label}:\n{url}\n\n"));
    }
    for paragraph in &content.after {
        text.push_str(paragraph);
        text.push_str("\n\n");
    }
    text.push_str(&format!("-- \n{}\n", footer_lines().join("\n")));
    text
}

/// The HTML part: Kura's colours on a plain table layout that mail programs show alike.
pub(crate) fn html(content: &Content) -> String {
    const P: &str = "margin:0 0 16px;font-size:16px;line-height:26px;color:#a1a1a1;";
    let last = content.paragraphs.len().saturating_sub(1);
    let body: String = content
        .paragraphs
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let strong = match (&content.emphasis, index == last) {
                (Some(emphasis), true) => format!(
                    " <strong style=\"color:#f5f5f5;font-weight:600;\">{}</strong>",
                    escape(emphasis)
                ),
                _ => String::new(),
            };
            format!("<p style=\"{P}\">{}{strong}</p>\n", escape(text))
        })
        .collect();
    let after: String = content
        .after
        .iter()
        .map(|text| format!("<p style=\"{P}\">{}</p>\n", escape(text)))
        .collect();
    let (button, fallback) = match &content.button {
        Some((label, url)) => (
            format!(
                "<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin:32px 0;\"><tr><td align=\"center\">\
                 <table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr>\
                 <td bgcolor=\"#8b7cff\" style=\"background-color:#8b7cff;border-radius:8px;\">\
                 <a href=\"{url}\" style=\"display:inline-block;padding:14px 28px;font-size:15px;line-height:20px;font-weight:600;color:#0a0a0a;text-decoration:none;border-radius:8px;\">{label}</a>\
                 </td></tr></table></td></tr></table>\n",
                url = escape(url),
                label = escape(label),
            ),
            format!(
                "<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin-top:16px;\"><tr><td style=\"border-top:1px solid #262626;padding-top:24px;\">\
                 <p style=\"margin:0 0 8px;font-size:13px;line-height:20px;color:#848484;\">If the button does not work, copy this address into your browser:</p>\
                 <p style=\"margin:0;font-family:'Geist Mono',ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;line-height:18px;word-break:break-all;\"><a href=\"{url}\" style=\"color:#8b7cff;text-decoration:none;\">{url}</a></p>\
                 </td></tr></table>\n",
                url = escape(url),
            ),
        ),
        None => (String::new(), String::new()),
    };
    LAYOUT
        .replace("{{subject}}", &escape(&content.subject))
        .replace("{{preview}}", &escape(&content.preview))
        .replace("{{greeting}}", &escape(&content.heading))
        .replace("{{body}}", &body)
        .replace("{{button}}", &button)
        .replace("{{after}}", &after)
        .replace("{{fallback}}", &fallback)
        .replace(
            "{{footer}}",
            &footer_lines()
                .iter()
                .map(|line| escape(line))
                .collect::<Vec<_>>()
                .join("<br>"),
        )
}

/// The lines under every mail.
fn footer_lines() -> [String; 2] {
    let year = chrono::Utc::now().format("%Y");
    [
        format!("© {year} Koxility · Kura"),
        "Sent automatically. Replies to this address are not read.".to_owned(),
    ]
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn transport(mail: &Mail) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    type Smtp = AsyncSmtpTransport<Tokio1Executor>;
    let failed = |err: lettre::transport::smtp::Error| {
        tracing::warn!("the mail server's name: {err}");
        CoreError::Mail("the mail server's name cannot be used for an encrypted connection".into())
    };
    let builder = match mail.encryption {
        MailEncryption::Starttls => Smtp::starttls_relay(&mail.host).map_err(failed)?,
        MailEncryption::Tls => Smtp::relay(&mail.host).map_err(failed)?,
        MailEncryption::None => Smtp::builder_dangerous(&mail.host),
    };
    let mut builder = builder.port(mail.port).timeout(Some(MAIL_TIMEOUT));
    if !mail.username.is_empty() {
        let credentials = Credentials::new(mail.username.clone(), mail.password.clone());
        builder = builder.credentials(credentials);
    }
    Ok(builder.build())
}

/// The mail server's complaint in words an admin can act on. Only fixed texts and the
/// server's three-digit code reach the browser, never what the server wrote: otherwise the
/// test mail would read out the greeting of whatever service runs at an address, and so map
/// the network Kura is in. The whole answer goes to the log, for whoever runs the server.
pub(crate) fn describe(err: &lettre::transport::smtp::Error) -> String {
    tracing::warn!("the mail server: {err}");
    if err.is_timeout() {
        return "the mail server did not answer in time".into();
    }
    if err.is_tls() || err.to_string().contains("certificate") {
        return "the encrypted connection to the mail server failed; check the encryption \
                setting and the server's certificate (details in Kura's log)"
            .into();
    }
    if let Some(code) = err.status() {
        let code = code.to_string();
        if code == "535" {
            return "the mail server refused the user name or password".into();
        }
        if err.is_transient() {
            return format!("the mail server refused for now (code {code}); try again later");
        }
        return format!("the mail server refused (code {code}); details in Kura's log");
    }
    if err.is_response() || err.is_client() {
        return "the server at this address did not answer like a mail server".into();
    }
    "could not connect to the mail server; check the address and the port".into()
}

#[cfg(test)]
mod tests {
    use kura_domain::Role;

    use super::*;

    fn settings() -> Mail {
        Mail {
            host: "smtp.example.org".into(),
            port: 587,
            encryption: MailEncryption::Starttls,
            username: "noreply@example.org".into(),
            password: "secret".into(),
            from: "Kura <noreply@example.org>".into(),
            public_url: "https://kura.example.org".into(),
        }
    }

    #[test]
    fn text_from_people_is_escaped_in_the_html() {
        let content = Content {
            subject: "s".into(),
            preview: "p".into(),
            heading: "h".into(),
            paragraphs: vec!["Hello <img src=x onerror=alert(1)> & \"friends\"".into()],
            emphasis: Some("<b>valid</b>".into()),
            button: Some((
                "Go".into(),
                "https://kura.example.org/reset-password#token=ab\"><x".into(),
            )),
            after: vec![],
        };
        let html = html(&content);
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt; &amp; &quot;friends&quot;"));
        assert!(!html.contains("ab\"><x"));
        assert!(!html.contains("{{"), "every placeholder is filled");
        assert!(html.contains("&lt;b&gt;valid&lt;/b&gt;</strong>"));
        let plain = plain(&content);
        assert!(plain.contains("Go:\nhttps://kura.example.org/reset-password#token=ab"));
    }

    #[test]
    fn a_name_cannot_add_headers_to_a_mail() {
        let user = UserEntry {
            id: 1,
            name: "sam\r\nBcc: thief@example.org".into(),
            role: Role::User,
            email: Some("sam@example.org".into()),
        };
        let to = Mailbox::new(Some(user.name.clone()), address("sam@example.org").unwrap());
        let content = Content {
            subject: "Kura: reset your password".into(),
            preview: String::new(),
            heading: String::new(),
            paragraphs: vec![],
            emphasis: None,
            button: None,
            after: vec![],
        };
        let message = message(&settings(), to, &content).unwrap();
        let raw = String::from_utf8(message.formatted()).unwrap();
        let headers = raw.split("\r\n\r\n").next().unwrap();
        assert!(
            !headers
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("bcc:")),
            "{headers}"
        );
        assert!(address("sam@example.org\r\nBcc: x@example.org").is_err());
    }

    #[test]
    fn settings_that_could_inject_or_leak_are_refused() {
        assert!(check_mail(&settings()).is_ok());
        assert!(
            check_mail(&Mail::default()).is_ok(),
            "empty switches mail off"
        );
        let refused = |change: fn(&mut Mail)| {
            let mut mail = settings();
            change(&mut mail);
            check_mail(&mail).is_err()
        };
        assert!(refused(
            |mail| mail.from = "Kura <a@example.org>\r\nBcc: b@example.org".into()
        ));
        assert!(refused(
            |mail| mail.host = "smtp.example.org\r\nRCPT TO:<x>".into()
        ));
        assert!(refused(|mail| mail.host = "smtp://smtp.example.org".into()));
        assert!(refused(|mail| mail.username = "a\r\nb".into()));
        assert!(refused(
            |mail| mail.public_url = "https://user:pw@kura.example.org".into()
        ));
        assert!(refused(
            |mail| mail.public_url = "https://kura.example.org/#x".into()
        ));
        assert!(refused(
            |mail| mail.public_url = "ftp://kura.example.org".into()
        ));
        assert!(refused(|mail| mail.encryption = MailEncryption::None));
        // A relay in the local network without a password is fine unencrypted.
        let mut relay = settings();
        relay.encryption = MailEncryption::None;
        relay.username.clear();
        relay.password.clear();
        assert!(check_mail(&relay).is_ok());
    }
}
