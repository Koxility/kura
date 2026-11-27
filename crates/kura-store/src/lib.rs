//! SQLite persistence: users, sessions, settings and the download queue.
//!
//! Finished jobs keep the path of their file, so the queue table is also the library index.

use std::{
    path::Path,
    str::FromStr,
    sync::{Mutex, MutexGuard, PoisonError},
};

use kura_domain::{
    Account, ApiKey, EpisodeId, Invite, Job, Language, Mail, Picked, Request, RequestState, Role,
    Settings, SiteId, Theme, User, UserEntry,
};
use rusqlite::{Connection, OptionalExtension, Row, functions::FunctionFlags, params, types::Type};
use rusqlite_migration::{M, Migrations};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("stored settings are not valid: {0}")]
    Settings(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

const SCHEMA: &str = "
CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL
);
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
CREATE TABLE settings (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    json TEXT NOT NULL
);
CREATE TABLE jobs (
    id           INTEGER PRIMARY KEY,
    site         TEXT NOT NULL,
    slug         TEXT NOT NULL,
    season       INTEGER NOT NULL,
    episode      INTEGER NOT NULL,
    series_title TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'queued',
    progress     REAL NOT NULL DEFAULT 0,
    error        TEXT,
    path         TEXT,
    language     TEXT,
    hoster       TEXT,
    dir          TEXT NOT NULL,
    hidden       INTEGER NOT NULL DEFAULT 0,
    queued_at    INTEGER NOT NULL,
    user_id      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE (site, slug, season, episode)
);
";

/// Added after the first release candidate; existing databases keep their data.
const REQUESTS_AND_IMDB: &str = "
ALTER TABLE jobs ADD COLUMN imdb_id TEXT;
CREATE TABLE requests (
    id           INTEGER PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    site         TEXT NOT NULL,
    slug         TEXT NOT NULL,
    series_title TEXT NOT NULL,
    picked       TEXT NOT NULL,
    language     TEXT,
    hoster       TEXT,
    imdb_id      TEXT,
    state        TEXT NOT NULL DEFAULT 'open',
    created_at   INTEGER NOT NULL,
    decided_at   INTEGER
);
";

/// Added in 1.2.10: when a worker last started a job, so Auto-Sync follows a title from its
/// first download on.
const STARTED_AT: &str = "ALTER TABLE jobs ADD COLUMN started_at INTEGER;";

/// Added in 1.3.0: an optional email and the theme per user, and API keys for other
/// programs such as Musubi.
const ACCOUNTS: &str = "
ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN theme TEXT;
CREATE TABLE api_keys (
    id           INTEGER PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    key_hash     TEXT NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);
";

/// Added in 1.4.0: the mail server for password resets, and the reset links that are out.
/// Only the SHA-256 of a link's token is kept.
const MAIL_AND_RESETS: &str = "
CREATE TABLE mail (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    json TEXT NOT NULL
);
CREATE TABLE password_resets (
    token_hash TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
";

/// Added in 1.4.0: whether an account was already asked for an email address after login.
const EMAIL_ASKED: &str = "ALTER TABLE users ADD COLUMN email_asked INTEGER NOT NULL DEFAULT 0;";

/// Added in 1.5.0: invitations by email. Only the SHA-256 of an invite link's token is kept; an
/// invite ends with the admin who sent it.
const INVITES: &str = "
CREATE TABLE invites (
    id         INTEGER PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    email      TEXT NOT NULL,
    role       TEXT NOT NULL,
    invited_by INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
";

/// Added in 1.5.2: a new email address waits here until the link mailed to it is opened.
/// Only the SHA-256 of the link's token is kept; one change per account at a time.
const EMAIL_CHANGES: &str = "
CREATE TABLE email_changes (
    user_id    INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    email      TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL
);
";

/// 1.5.3: the files an Auto-Sync download replaces once it is done, and what the last
/// Auto-Sync check of each title found, kept over a restart.
const AUTO_SYNC_STATE: &str = "
ALTER TABLE jobs ADD COLUMN replaces TEXT;
CREATE TABLE sync_titles (
    site   TEXT NOT NULL,
    slug   TEXT NOT NULL,
    result TEXT NOT NULL,
    PRIMARY KEY (site, slug)
);
CREATE TABLE sync_runs (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    last_run INTEGER NOT NULL
);
";

/// What became of an invite link that someone used to create an account.
#[derive(Debug, PartialEq)]
pub enum Redeemed {
    Created(User),
    /// Another account has the name; the invite still works with another one.
    NameTaken,
    /// Another account has the invite's email address by now.
    EmailTaken,
    /// No such invite, used up, or expired.
    Invalid,
}

/// What became of opening the link that confirms a new email address.
#[derive(Debug, PartialEq)]
pub enum EmailConfirmed {
    /// The account's email is now this address.
    Changed { user_id: i64, email: String },
    /// Another account or an invite has the address by now; the link is used up.
    Taken,
    /// No such link, used up, or expired.
    Invalid,
}

const JOB_COLUMNS: &str = "id, site, slug, season, episode, series_title, state, progress, error, path, \
     language, hoster, dir, hidden, queued_at, imdb_id, started_at, replaces";

const REQUEST_COLUMNS: &str = "r.id, r.user_id, u.name, r.site, r.slug, r.series_title, r.picked, \
     r.language, r.hoster, r.imdb_id, r.state, r.created_at";

/// A request as a user sends it.
pub struct NewRequest<'a> {
    pub user_id: i64,
    pub site: kura_domain::SiteId,
    pub slug: &'a str,
    pub series_title: &'a str,
    pub picked: &'a Picked,
    pub language: Option<Language>,
    pub hoster: Option<&'a str>,
    pub imdb_id: Option<&'a str>,
    /// Unix seconds.
    pub now: i64,
}

/// An episode to queue, with what the user chose for it.
pub struct NewJob<'a> {
    pub episode: &'a EpisodeId,
    pub series_title: &'a str,
    /// `None` when Auto-Sync queued it.
    pub user_id: Option<i64>,
    /// `None` leaves the choice to the settings.
    pub language: Option<Language>,
    /// `None` leaves the choice to the settings.
    pub hoster: Option<&'a str>,
    /// Folder the `Series` folder of this download lives in.
    pub dir: &'a str,
    /// For the folder name; `None` only for downloads that keep an old folder name.
    pub imdb_id: Option<&'a str>,
    /// Unix seconds.
    pub now: i64,
    /// Also queue it again when it is done: its file is gone, or Auto-Sync found it in
    /// another language than the one it follows. Auto-Sync never asks for an episode whose
    /// file someone deleted.
    pub replace_done: bool,
    /// Files this download replaces: deleted once it is done. Empty for most.
    pub replaces: &'a [String],
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // SQLite's own `lower()` folds only A to Z; email addresses are compared with this,
        // so `Ärger@example.org` and `ärger@example.org` are one address too.
        conn.create_scalar_function(
            "casefold",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                Ok(ctx
                    .get::<Option<String>>(0)?
                    .map(|text| text.to_lowercase()))
            },
        )?;
        Migrations::new(vec![
            M::up(SCHEMA),
            M::up(REQUESTS_AND_IMDB),
            M::up(STARTED_AT),
            M::up(ACCOUNTS),
            M::up(MAIL_AND_RESETS),
            M::up(EMAIL_ASKED),
            M::up(INVITES),
            M::up(EMAIL_CHANGES),
            M::up(AUTO_SYNC_STATE),
        ])
        .to_latest(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // Every method is a single statement, so a panic elsewhere leaves nothing half-written.
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Users

    /// Returns `None` when the name is already taken. Names compare case-insensitively.
    pub fn create_user(&self, name: &str, password_hash: &str, role: Role) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "INSERT INTO users (name, password_hash, role) VALUES (?1, ?2, ?3)
                 ON CONFLICT (name) DO NOTHING RETURNING id, name, role",
                params![name, password_hash, role.as_str()],
                user,
            )
            .optional()?)
    }

    /// Creates the admin of a fresh installation. Returns `None` once any user exists.
    pub fn create_first_admin(&self, name: &str, password_hash: &str) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "INSERT INTO users (name, password_hash, role)
                 SELECT ?1, ?2, 'admin' WHERE NOT EXISTS (SELECT 1 FROM users)
                 RETURNING id, name, role",
                params![name, password_hash],
                user,
            )
            .optional()?)
    }

    pub fn has_users(&self) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?)
    }

    /// The user and their password hash, for checking a login.
    pub fn credentials(&self, name: &str) -> Result<Option<(User, String)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, role, password_hash FROM users WHERE name = ?1",
                [name],
                |row| Ok((user(row)?, row.get(3)?)),
            )
            .optional()?)
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, role FROM users ORDER BY id")?;
        let users = stmt.query_map([], user)?.collect::<rusqlite::Result<_>>()?;
        Ok(users)
    }

    /// Returns whether a user was deleted. The last admin stays, so nobody gets locked out.
    pub fn delete_user(&self, id: i64) -> Result<bool> {
        let deleted = self.conn().execute(
            "DELETE FROM users WHERE id = ?1 AND (role != 'admin'
                 OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            [id],
        )?;
        Ok(deleted > 0)
    }

    /// A user's own account: name, role, email and theme.
    pub fn account(&self, id: i64) -> Result<Option<Account>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, role, email, theme, email_asked,
                     (SELECT c.email FROM email_changes c
                      WHERE c.user_id = users.id AND c.expires_at > unixepoch())
                 FROM users WHERE id = ?1",
                [id],
                |row| {
                    let theme: Option<String> = row.get(4)?;
                    Ok(Account {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role: text(row, 2)?,
                        email: row.get(3)?,
                        // Missing fields fall back to the defaults.
                        theme: theme
                            .and_then(|json| serde_json::from_str(&json).ok())
                            .unwrap_or_default(),
                        pending_email: row.get(6)?,
                        ask_email: row.get::<_, Option<String>>(3)?.is_none()
                            && !row.get::<_, bool>(5)?
                            && row.get::<_, Option<String>>(6)?.is_none(),
                    })
                },
            )
            .optional()?)
    }

    /// Returns `false` when another user has the name. Names compare case-insensitively.
    pub fn rename_user(&self, id: i64, name: &str) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE users SET name = ?2 WHERE id = ?1
             AND NOT EXISTS (SELECT 1 FROM users WHERE name = ?2 AND id != ?1)",
            params![id, name],
        )?;
        Ok(changed > 0)
    }

    /// Sets or clears a user's email at once. `false` when another account or an invite that
    /// still works has the address (in any case). Also ends the reset links that are out (they
    /// went to the old address) and a change that waits for confirmation.
    pub fn set_email(&self, id: i64, email: Option<&str>, now: i64) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if let Some(email) = email
            && (account_has_email(&tx, email, Some(id))? || invited(&tx, email, now)?)
        {
            return Ok(false);
        }
        tx.execute(
            "UPDATE users SET email = ?2, email_asked = 1 WHERE id = ?1",
            params![id, email],
        )?;
        tx.execute("DELETE FROM password_resets WHERE user_id = ?1", [id])?;
        tx.execute("DELETE FROM email_changes WHERE user_id = ?1", [id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Whether another account than `except` has this email, or an invite that still works.
    pub fn email_taken(&self, email: &str, except: Option<i64>, now: i64) -> Result<bool> {
        let conn = self.conn();
        Ok(account_has_email(&conn, email, except)? || invited(&conn, email, now)?)
    }

    // Email changes. Times are Unix seconds; the caller hashes the token.

    /// Keeps a new address for a user until its link is opened; replaces a change that was
    /// waiting. `false` when another account or an invite has the address.
    pub fn start_email_change(
        &self,
        user_id: i64,
        email: &str,
        token_hash: &str,
        now: i64,
        ttl: i64,
    ) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if account_has_email(&tx, email, Some(user_id))? || invited(&tx, email, now)? {
            return Ok(false);
        }
        tx.execute("DELETE FROM email_changes WHERE expires_at <= ?1", [now])?;
        tx.execute(
            "INSERT INTO email_changes (user_id, email, token_hash, expires_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (user_id) DO UPDATE SET email = excluded.email,
                 token_hash = excluded.token_hash, expires_at = excluded.expires_at",
            params![user_id, email, token_hash, now + ttl],
        )?;
        tx.execute("UPDATE users SET email_asked = 1 WHERE id = ?1", [user_id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Forgets the change that waits for a user's confirmation, if any.
    pub fn cancel_email_change(&self, user_id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM email_changes WHERE user_id = ?1", [user_id])?;
        Ok(())
    }

    /// Uses up the link that confirms a new address, in one transaction: the link must still
    /// work, and the address must still be free, then it becomes the account's email and the
    /// reset links to the old address end.
    pub fn confirm_email_change(&self, token_hash: &str, now: i64) -> Result<EmailConfirmed> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let found = tx
            .query_row(
                "DELETE FROM email_changes WHERE token_hash = ?1 AND expires_at > ?2
                 RETURNING user_id, email",
                params![token_hash, now],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        tx.execute("DELETE FROM email_changes WHERE expires_at <= ?1", [now])?;
        let Some((user_id, email)) = found else {
            tx.commit()?;
            return Ok(EmailConfirmed::Invalid);
        };
        if account_has_email(&tx, &email, Some(user_id))? || invited(&tx, &email, now)? {
            tx.commit()?;
            return Ok(EmailConfirmed::Taken);
        }
        tx.execute(
            "UPDATE users SET email = ?2, email_asked = 1 WHERE id = ?1",
            params![user_id, email],
        )?;
        tx.execute("DELETE FROM password_resets WHERE user_id = ?1", [user_id])?;
        tx.commit()?;
        Ok(EmailConfirmed::Changed { user_id, email })
    }

    // Invites. Times are Unix seconds; the caller hashes the token.

    /// Stores an invite. An older invite for the same address stops working.
    #[allow(clippy::too_many_arguments)]
    pub fn create_invite(
        &self,
        token_hash: &str,
        email: &str,
        role: Role,
        invited_by: i64,
        now: i64,
        ttl: i64,
    ) -> Result<Invite> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM invites WHERE casefold(email) = casefold(?1) OR expires_at <= ?2",
            params![email, now],
        )?;
        Ok(conn.query_row(
            "INSERT INTO invites (token_hash, email, role, invited_by, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             RETURNING id, email, role, created_at, expires_at",
            params![token_hash, email, role.as_str(), invited_by, now, now + ttl],
            invite,
        )?)
    }

    /// The invites that still work.
    pub fn invites(&self, now: i64) -> Result<Vec<Invite>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, email, role, created_at, expires_at FROM invites
             WHERE expires_at > ?1 ORDER BY id",
        )?;
        let invites = stmt
            .query_map([now], invite)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(invites)
    }

    /// The invite a link names, if it still works.
    pub fn invite_by_token(&self, token_hash: &str, now: i64) -> Result<Option<Invite>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, email, role, created_at, expires_at FROM invites
                 WHERE token_hash = ?1 AND expires_at > ?2",
                params![token_hash, now],
                invite,
            )
            .optional()?)
    }

    pub fn delete_invite(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM invites WHERE id = ?1", [id])?
            > 0)
    }

    /// Whether an account already has this email (case-insensitive).
    pub fn email_in_use(&self, email: &str) -> Result<bool> {
        Ok(account_has_email(&self.conn(), email, None)?)
    }

    /// Creates the account an invite link stands for, in one transaction: the invite must still
    /// work, the name must be free, and the invite is gone afterwards. Email and role come from
    /// the invite, never from the person using it.
    pub fn redeem_invite(
        &self,
        token_hash: &str,
        name: &str,
        password_hash: &str,
        now: i64,
    ) -> Result<Redeemed> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let found = tx
            .query_row(
                "SELECT id, email, role FROM invites WHERE token_hash = ?1 AND expires_at > ?2",
                params![token_hash, now],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        text::<Role>(row, 2)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, email, role)) = found else {
            return Ok(Redeemed::Invalid);
        };
        // An account may have taken the address since the invite was sent.
        if account_has_email(&tx, &email, None)? {
            return Ok(Redeemed::EmailTaken);
        }
        let created = tx
            .query_row(
                "INSERT INTO users (name, password_hash, role, email, email_asked)
                 VALUES (?1, ?2, ?3, ?4, 1)
                 ON CONFLICT (name) DO NOTHING RETURNING id, name, role",
                params![name, password_hash, role.as_str(), email],
                user,
            )
            .optional()?;
        let Some(created) = created else {
            return Ok(Redeemed::NameTaken);
        };
        tx.execute("DELETE FROM invites WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(Redeemed::Created(created))
    }

    /// The account was asked for an email after login and said "not now"; it is not asked again.
    pub fn mark_email_asked(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("UPDATE users SET email_asked = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Every user with their email, for admins.
    pub fn user_entries(&self) -> Result<Vec<UserEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, role, email FROM users ORDER BY id")?;
        let users = stmt
            .query_map([], user_entry)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(users)
    }

    /// The users a reset was asked for: by name, or by email (both case-insensitive).
    pub fn users_named(&self, name_or_email: &str) -> Result<Vec<UserEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, role, email FROM users
             WHERE name = ?1 OR (email IS NOT NULL AND casefold(email) = casefold(?1))
             ORDER BY id",
        )?;
        let users = stmt
            .query_map([name_or_email], user_entry)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(users)
    }

    // Password resets. Times are Unix seconds; the caller hashes the token.

    /// Stores a new reset link for a user; one sent before stops working.
    pub fn create_reset(&self, token_hash: &str, user_id: i64, now: i64, ttl: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM password_resets WHERE user_id = ?1 OR expires_at <= ?2",
            params![user_id, now],
        )?;
        conn.execute(
            "INSERT INTO password_resets (token_hash, user_id, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, user_id, now + ttl, now],
        )?;
        Ok(())
    }

    /// When the user's newest reset link was made, if one is still out.
    pub fn last_reset(&self, user_id: i64, now: i64) -> Result<Option<i64>> {
        Ok(self.conn().query_row(
            "SELECT max(created_at) FROM password_resets WHERE user_id = ?1 AND expires_at > ?2",
            params![user_id, now],
            |row| row.get(0),
        )?)
    }

    /// Uses up a reset link: returns its user and deletes it in one step, so a link works once
    /// even when it is sent twice at the same moment. `None` for an unknown or expired link.
    pub fn take_reset(&self, token_hash: &str, now: i64) -> Result<Option<i64>> {
        let conn = self.conn();
        let user = conn
            .query_row(
                "DELETE FROM password_resets WHERE token_hash = ?1 AND expires_at > ?2
                 RETURNING user_id",
                params![token_hash, now],
                |row| row.get(0),
            )
            .optional()?;
        conn.execute("DELETE FROM password_resets WHERE expires_at <= ?1", [now])?;
        Ok(user)
    }

    // Mail

    /// The mail server settings; empty while none were saved.
    pub fn mail(&self) -> Result<Mail> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM mail WHERE id = 1", [], |row| row.get(0))
            .optional()?;
        Ok(match json {
            Some(json) => serde_json::from_str(&json)?,
            None => Mail::default(),
        })
    }

    pub fn save_mail(&self, mail: &Mail) -> Result<()> {
        self.conn().execute(
            "INSERT INTO mail (id, json) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            [serde_json::to_string(mail)?],
        )?;
        Ok(())
    }

    pub fn set_theme(&self, id: i64, theme: &Theme) -> Result<()> {
        self.conn().execute(
            "UPDATE users SET theme = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(theme)?],
        )?;
        Ok(())
    }

    // API keys. The caller hashes the key.

    pub fn create_api_key(
        &self,
        user_id: i64,
        name: &str,
        key_hash: &str,
        now: i64,
    ) -> Result<ApiKey> {
        Ok(self.conn().query_row(
            "INSERT INTO api_keys (user_id, name, key_hash, created_at) VALUES (?1, ?2, ?3, ?4)
             RETURNING id, name, created_at, last_used_at",
            params![user_id, name, key_hash, now],
            api_key,
        )?)
    }

    pub fn api_keys(&self, user_id: i64) -> Result<Vec<ApiKey>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at, last_used_at FROM api_keys WHERE user_id = ?1
             ORDER BY id",
        )?;
        let keys = stmt
            .query_map([user_id], api_key)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(keys)
    }

    /// Returns whether one of the user's keys was deleted.
    pub fn delete_api_key(&self, id: i64, user_id: i64) -> Result<bool> {
        let deleted = self.conn().execute(
            "DELETE FROM api_keys WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(deleted > 0)
    }

    /// The user a key belongs to; notes when it was used.
    pub fn api_key_user(&self, key_hash: &str, now: i64) -> Result<Option<User>> {
        let conn = self.conn();
        let found = conn
            .query_row(
                "SELECT u.id, u.name, u.role FROM api_keys k JOIN users u ON u.id = k.user_id
                 WHERE k.key_hash = ?1",
                [key_hash],
                user,
            )
            .optional()?;
        if found.is_some() {
            conn.execute(
                "UPDATE api_keys SET last_used_at = ?2 WHERE key_hash = ?1",
                params![key_hash, now],
            )?;
        }
        Ok(found)
    }

    // Sessions. Times are Unix seconds; the caller hashes the token.

    pub fn create_session(&self, token_hash: &str, user_id: i64, now: i64, ttl: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])?;
        conn.execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![token_hash, user_id, now + ttl],
        )?;
        Ok(())
    }

    pub fn session_user(&self, token_hash: &str, now: i64) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT u.id, u.name, u.role FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                params![token_hash, now],
                user,
            )
            .optional()?)
    }

    /// Replaces a user's password hash, ends their other sessions and deletes their API
    /// keys and reset links, so a changed password also locks out every other device and
    /// program.
    pub fn set_password(
        &self,
        user_id: i64,
        password_hash: &str,
        keep_session: &str,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE users SET password_hash = ?2 WHERE id = ?1",
            params![user_id, password_hash],
        )?;
        conn.execute(
            "DELETE FROM sessions WHERE user_id = ?1 AND token_hash != ?2",
            params![user_id, keep_session],
        )?;
        conn.execute("DELETE FROM api_keys WHERE user_id = ?1", [user_id])?;
        conn.execute("DELETE FROM password_resets WHERE user_id = ?1", [user_id])?;
        Ok(())
    }

    /// When a session ends (Unix seconds); with its length, when it began.
    pub fn session_expiry(&self, token_hash: &str) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT expires_at FROM sessions WHERE token_hash = ?1",
                [token_hash],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn delete_session(&self, token_hash: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM sessions WHERE token_hash = ?1", [token_hash])?;
        Ok(())
    }

    // Settings

    /// The saved settings, or the defaults while nothing was saved yet.
    pub fn settings(&self) -> Result<Settings> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(match json {
            Some(json) => serde_json::from_value(upgrade(serde_json::from_str(&json)?))?,
            None => Settings::default(),
        })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    // Jobs

    /// Queues an episode and returns the job id. A failed or cancelled job for the same
    /// episode is queued again with the new choices, a done one too with `replace_done`;
    /// `None` means it is already queued, running or done.
    pub fn enqueue(&self, new: &NewJob) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "INSERT INTO jobs (site, slug, season, episode, series_title, user_id,
                                   language, hoster, dir, queued_at, imdb_id, replaces)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?13)
                 ON CONFLICT (site, slug, season, episode) DO UPDATE
                     SET state = 'queued', progress = 0, error = NULL, hidden = 0,
                         user_id = excluded.user_id, language = excluded.language,
                         hoster = excluded.hoster, dir = excluded.dir,
                         queued_at = excluded.queued_at, imdb_id = excluded.imdb_id,
                         replaces = excluded.replaces
                     WHERE state IN ('failed', 'cancelled') OR (?12 AND state = 'done')
                 RETURNING id",
                params![
                    new.episode.site.as_str(),
                    new.episode.slug,
                    new.episode.season,
                    new.episode.number,
                    new.series_title,
                    new.user_id,
                    new.language.map(Language::as_str),
                    new.hoster,
                    new.dir,
                    new.now,
                    new.imdb_id,
                    new.replace_done,
                    (!new.replaces.is_empty())
                        .then(|| serde_json::to_string(new.replaces))
                        .transpose()?
                ],
                |row| row.get(0),
            )
            .optional()?)
    }

    // Requests

    pub fn create_request(&self, new: &NewRequest) -> Result<i64> {
        Ok(self.conn().query_row(
            "INSERT INTO requests (user_id, site, slug, series_title, picked, language, hoster,
                                   imdb_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
            params![
                new.user_id,
                new.site.as_str(),
                new.slug,
                new.series_title,
                serde_json::to_string(new.picked)?,
                new.language.map(Language::as_str),
                new.hoster,
                new.imdb_id,
                new.now
            ],
            |row| row.get(0),
        )?)
    }

    /// Open requests first, then the newest. With `user`, only that user's requests.
    pub fn requests(&self, user: Option<i64>) -> Result<Vec<Request>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {REQUEST_COLUMNS} FROM requests r JOIN users u ON u.id = r.user_id
             WHERE ?1 IS NULL OR r.user_id = ?1
             ORDER BY r.state != 'open', r.id DESC"
        ))?;
        let requests = stmt
            .query_map([user], request)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(requests)
    }

    /// Moves an open request to `state` and returns it; `None` when it was not open, so two
    /// admins deciding at once cannot both act on it.
    pub fn decide_request(
        &self,
        id: i64,
        state: RequestState,
        now: i64,
    ) -> Result<Option<Request>> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE requests SET state = ?2, decided_at = ?3 WHERE id = ?1 AND state = 'open'",
            params![id, state.as_str(), now],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {REQUEST_COLUMNS} FROM requests r JOIN users u ON u.id = r.user_id
                     WHERE r.id = ?1"
                ),
                [id],
                request,
            )
            .optional()?)
    }

    /// Deletes a user's own request while nobody has decided on it. Returns whether it did.
    pub fn cancel_request(&self, id: i64, user_id: i64) -> Result<bool> {
        let deleted = self.conn().execute(
            "DELETE FROM requests WHERE id = ?1 AND user_id = ?2 AND state = 'open'",
            params![id, user_id],
        )?;
        Ok(deleted > 0)
    }

    /// Deletes accepted and rejected requests; open ones stay. Returns how many.
    pub fn clear_decided_requests(&self) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM requests WHERE state != 'open'", [])?)
    }

    /// Puts a request back to open, when accepting it failed half-way.
    pub fn reopen_request(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE requests SET state = 'open', decided_at = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    // Auto-Sync

    /// Keeps what the last Auto-Sync check of a title found, as JSON.
    pub fn save_sync_result(&self, site: SiteId, slug: &str, result: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_titles (site, slug, result) VALUES (?1, ?2, ?3)
             ON CONFLICT (site, slug) DO UPDATE SET result = excluded.result",
            params![site.as_str(), slug, result],
        )?;
        Ok(())
    }

    /// What the last Auto-Sync check of each title found, as saved.
    pub fn sync_results(&self) -> Result<Vec<(SiteId, String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT site, slug, result FROM sync_titles")?;
        let rows = stmt.query_map([], |row| Ok((text(row, 0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_last_sync_run(&self, at: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_runs (id, last_run) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET last_run = excluded.last_run",
            [at],
        )?;
        Ok(())
    }

    /// Unix seconds of the last Auto-Sync run, also from before a restart.
    pub fn last_sync_run(&self) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row("SELECT last_run FROM sync_runs WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Takes finished, failed and cancelled downloads off the queue page. Returns how many.
    pub fn hide_finished(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE jobs SET hidden = 1
             WHERE hidden = 0 AND state IN ('done', 'failed', 'cancelled')",
            [],
        )?)
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {JOB_COLUMNS} FROM jobs ORDER BY id"))?;
        let jobs = stmt.query_map([], job)?.collect::<rusqlite::Result<_>>()?;
        Ok(jobs)
    }

    /// Marks the oldest queued job as running, notes when, and returns it.
    pub fn claim_next(&self) -> Result<Option<Job>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "UPDATE jobs SET state = 'running', started_at = unixepoch()
                     WHERE id = (SELECT id FROM jobs WHERE state = 'queued' ORDER BY id LIMIT 1)
                     RETURNING {JOB_COLUMNS}"
                ),
                [],
                job,
            )
            .optional()?)
    }

    pub fn set_progress(&self, id: i64, progress: f64) -> Result<()> {
        self.update_running("progress = ?2", params![id, progress])
    }

    /// Marks a job done with its file and the language that was downloaded, which replaces
    /// the picked one (`None` for "follow the settings").
    pub fn finish(&self, id: i64, path: &str, language: Language) -> Result<()> {
        self.update_running(
            "state = 'done', progress = 1, path = ?2, language = ?3, error = NULL",
            params![id, path, language.as_str()],
        )
    }

    /// Why a running download starts again, shown until it ends.
    pub fn note_retry(&self, id: i64, note: &str) -> Result<()> {
        self.update_running("progress = 0, error = ?2", params![id, note])
    }

    pub fn fail(&self, id: i64, error: &str) -> Result<()> {
        self.update_running("state = 'failed', error = ?2", params![id, error])
    }

    /// Only a running job changes, so a late result from a worker never revives a cancelled job.
    fn update_running(&self, set: &str, params: &[&dyn rusqlite::ToSql]) -> Result<()> {
        self.conn().execute(
            &format!("UPDATE jobs SET {set} WHERE id = ?1 AND state = 'running'"),
            params,
        )?;
        Ok(())
    }

    /// Returns whether a queued or running job was cancelled.
    pub fn cancel(&self, id: i64) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE jobs SET state = 'cancelled' WHERE id = ?1 AND state IN ('queued', 'running')",
            [id],
        )?;
        Ok(changed > 0)
    }

    /// Call once at startup: jobs that were running when the server stopped start over.
    pub fn requeue_interrupted(&self) -> Result<()> {
        self.conn().execute(
            "UPDATE jobs SET state = 'queued', progress = 0 WHERE state = 'running'",
            [],
        )?;
        Ok(())
    }
}

/// Whether an account other than `except` has this email, in any case.
fn account_has_email(
    conn: &Connection,
    email: &str,
    except: Option<i64>,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM users WHERE email IS NOT NULL
             AND casefold(email) = casefold(?1) AND id IS NOT ?2)",
        params![email, except],
        |row| row.get(0),
    )
}

/// Whether an invite that still works was sent to this email, in any case.
fn invited(conn: &Connection, email: &str, now: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM invites
             WHERE casefold(email) = casefold(?1) AND expires_at > ?2)",
        params![email, now],
        |row| row.get(0),
    )
}

/// Settings saved before the download path became a library: the download path becomes the
/// main library, so downloads keep going where they went.
fn upgrade(mut settings: serde_json::Value) -> serde_json::Value {
    let Some(object) = settings.as_object_mut() else {
        return settings;
    };
    let enabled = object.remove("library_enabled").and_then(|on| on.as_bool());
    let Some(dir) = object.remove("download_dir") else {
        return settings;
    };
    let Some(dir) = dir.as_str().filter(|dir| !dir.is_empty()) else {
        return settings;
    };
    // The old default was relative to the folder the server runs in.
    let dir = std::path::absolute(dir).map_or(dir.into(), |dir| dir.to_string_lossy().into_owned());

    let mut libraries = match object.remove("libraries") {
        Some(serde_json::Value::Array(libraries)) => libraries,
        _ => Vec::new(),
    };
    if enabled != Some(true) {
        // The libraries were switched off, so their site defaults did not apply.
        for library in &mut libraries {
            library["default_for"] = serde_json::json!([]);
        }
    }
    let same_path = |library: &serde_json::Value| library["path"].as_str() == Some(dir.as_str());
    if !libraries.iter().any(same_path) {
        let taken = |name: &str| {
            libraries.iter().any(|library| {
                let other = library["name"].as_str().unwrap_or_default();
                other.eq_ignore_ascii_case(name)
            })
        };
        let name = ["Downloads", "Downloads (old)", "Main"]
            .into_iter()
            .find(|name| !taken(name))
            .unwrap_or("Download path");
        let main = serde_json::json!({ "name": name, "path": dir, "default_for": [] });
        libraries.insert(0, main);
    }
    object.insert("libraries".to_owned(), serde_json::Value::Array(libraries));
    settings
}

fn text<T>(row: &Row, idx: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    row.get::<_, String>(idx)?
        .parse()
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(err)))
}

fn user(row: &Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        name: row.get(1)?,
        role: text(row, 2)?,
    })
}

fn invite(row: &Row) -> rusqlite::Result<Invite> {
    Ok(Invite {
        id: row.get(0)?,
        email: row.get(1)?,
        role: text(row, 2)?,
        created_at: row.get(3)?,
        expires_at: row.get(4)?,
    })
}

fn user_entry(row: &Row) -> rusqlite::Result<UserEntry> {
    Ok(UserEntry {
        id: row.get(0)?,
        name: row.get(1)?,
        role: text(row, 2)?,
        email: row.get(3)?,
    })
}

fn api_key(row: &Row) -> rusqlite::Result<ApiKey> {
    Ok(ApiKey {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        last_used_at: row.get(3)?,
    })
}

fn job(row: &Row) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        episode: EpisodeId {
            site: text(row, 1)?,
            slug: row.get(2)?,
            season: row.get(3)?,
            number: row.get(4)?,
        },
        series_title: row.get(5)?,
        state: text(row, 6)?,
        progress: row.get(7)?,
        error: row.get(8)?,
        path: row.get(9)?,
        language: match row.get::<_, Option<String>>(10)? {
            Some(_) => Some(text(row, 10)?),
            None => None,
        },
        hoster: row.get(11)?,
        dir: row.get(12)?,
        hidden: row.get(13)?,
        queued_at: row.get(14)?,
        imdb_id: row.get(15)?,
        started_at: row.get(16)?,
        replaces: match row.get::<_, Option<String>>(17)? {
            Some(json) => serde_json::from_str(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(17, Type::Text, Box::new(err))
            })?,
            None => Vec::new(),
        },
    })
}

fn optional_text<T>(row: &Row, idx: usize) -> rusqlite::Result<Option<T>>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match row.get::<_, Option<String>>(idx)? {
        Some(_) => Ok(Some(text(row, idx)?)),
        None => Ok(None),
    }
}

fn request(row: &Row) -> rusqlite::Result<Request> {
    let picked: String = row.get(6)?;
    let picked = serde_json::from_str(&picked)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(err)))?;
    Ok(Request {
        id: row.get(0)?,
        user_id: row.get(1)?,
        user_name: row.get(2)?,
        site: text(row, 3)?,
        slug: row.get(4)?,
        series_title: row.get(5)?,
        picked,
        language: optional_text(row, 7)?,
        hoster: row.get(8)?,
        imdb_id: row.get(9)?,
        state: text(row, 10)?,
        created_at: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use kura_domain::{JobState, SiteId};

    use super::*;

    fn episode(number: u32) -> EpisodeId {
        EpisodeId {
            site: SiteId::AniWorld,
            slug: "frieren".into(),
            season: 1,
            number,
        }
    }

    fn new_job(episode: &EpisodeId, user_id: i64) -> NewJob<'_> {
        NewJob {
            episode,
            series_title: "Frieren",
            user_id: Some(user_id),
            language: Some(Language::GermanSub),
            hoster: None,
            dir: "/media",
            imdb_id: Some("tt0388629"),
            now: 100,
            replace_done: false,
            replaces: &[],
        }
    }

    fn store_with_admin() -> (Store, User) {
        let store = Store::open_in_memory().unwrap();
        let admin = store
            .create_user("admin", "hash", Role::Admin)
            .unwrap()
            .unwrap();
        (store, admin)
    }

    #[test]
    fn settings_and_users_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kura.db");

        let store = Store::open(&path).unwrap();
        assert_eq!(store.settings().unwrap(), Settings::default());
        let changed = Settings {
            parallel: 2,
            backdrop: kura_domain::Backdrop::Halo,
            ..Settings::default()
        };
        store.save_settings(&changed).unwrap();
        store.create_user("admin", "hash", Role::Admin).unwrap();
        drop(store);

        let store = Store::open(&path).unwrap();
        assert_eq!(store.settings().unwrap(), changed);
        let (user, hash) = store.credentials("ADMIN").unwrap().unwrap();
        assert_eq!((user.role, hash.as_str()), (Role::Admin, "hash"));
    }

    #[test]
    fn an_old_download_path_becomes_the_main_library() {
        let store = Store::open_in_memory().unwrap();
        let save = |json: &str| {
            let conn = store.conn();
            conn.execute(
                "INSERT OR REPLACE INTO settings (id, json) VALUES (1, ?1)",
                [json],
            )
            .unwrap();
        };
        let libraries = |store: &Store| -> Vec<(String, String, usize)> {
            let settings = store.settings().unwrap();
            let libraries = settings.libraries.into_iter();
            let summary = |library: kura_domain::Library| {
                let path = library.path.to_string_lossy().into_owned();
                (library.name, path, library.default_for.len())
            };
            libraries.map(summary).collect()
        };

        save(r#"{"download_dir": "/srv/dl", "parallel": 2}"#);
        assert_eq!(
            libraries(&store),
            [("Downloads".into(), "/srv/dl".into(), 0)]
        );
        assert_eq!(store.settings().unwrap().parallel, 2);

        // Libraries that were on keep their site defaults; the download path goes first.
        save(
            r#"{"download_dir": "/srv/dl", "library_enabled": true,
                "libraries": [{"name": "Downloads", "path": "/a", "default_for": ["aniworld"]}]}"#,
        );
        assert_eq!(
            libraries(&store),
            [
                ("Downloads (old)".into(), "/srv/dl".into(), 0),
                ("Downloads".into(), "/a".into(), 1),
            ]
        );

        // Libraries that were off lose their defaults, so everything still goes to one place.
        save(
            r#"{"download_dir": "/a", "library_enabled": false,
                "libraries": [{"name": "Anime", "path": "/a", "default_for": ["aniworld"]}]}"#,
        );
        assert_eq!(libraries(&store), [("Anime".into(), "/a".into(), 0)]);

        // A relative path is made absolute against the folder the server runs in.
        save(r#"{"download_dir": "downloads"}"#);
        let path = &libraries(&store)[0].1;
        assert!(std::path::Path::new(path).is_absolute() && path.ends_with("downloads"));

        // Settings saved after the change are read as they are.
        let current = Settings::default();
        store.save_settings(&current).unwrap();
        assert_eq!(store.settings().unwrap(), current);
    }

    #[test]
    fn only_a_fresh_installation_gets_a_first_admin() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.has_users().unwrap());
        let admin = store.create_first_admin("root", "hash").unwrap().unwrap();
        assert_eq!(admin.role, Role::Admin);
        assert!(store.has_users().unwrap());
        assert!(
            store
                .create_first_admin("intruder", "hash")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn user_names_are_unique_and_the_last_admin_stays() {
        let (store, admin) = store_with_admin();
        assert!(
            store
                .create_user("Admin", "x", Role::User)
                .unwrap()
                .is_none()
        );
        assert!(!store.delete_user(admin.id).unwrap());

        let second = store
            .create_user("second", "x", Role::Admin)
            .unwrap()
            .unwrap();
        assert!(store.delete_user(admin.id).unwrap());
        assert_eq!(store.users().unwrap(), vec![second]);
    }

    #[test]
    fn sessions_expire_and_die_with_their_user() {
        let (store, admin) = store_with_admin();
        let user = store.create_user("u", "x", Role::User).unwrap().unwrap();
        store.create_session("a", admin.id, 100, 10).unwrap();
        store.create_session("u", user.id, 100, 10).unwrap();

        assert_eq!(store.session_user("a", 109).unwrap(), Some(admin));
        assert_eq!(store.session_user("a", 110).unwrap(), None);

        assert!(store.delete_user(user.id).unwrap());
        assert_eq!(store.session_user("u", 101).unwrap(), None);
    }

    #[test]
    fn queue_hands_out_jobs_in_order_and_skips_duplicates() {
        let (store, admin) = store_with_admin();
        let first = store
            .enqueue(&new_job(&episode(1), admin.id))
            .unwrap()
            .unwrap();
        store.enqueue(&new_job(&episode(2), admin.id)).unwrap();
        assert_eq!(
            store.enqueue(&new_job(&episode(1), admin.id)).unwrap(),
            None
        );

        let job = store.claim_next().unwrap().unwrap();
        assert_eq!(
            (job.id, job.state, &job.episode),
            (first, JobState::Running, &episode(1))
        );
        assert_eq!(store.claim_next().unwrap().unwrap().episode, episode(2));
        assert!(store.claim_next().unwrap().is_none());
    }

    #[test]
    fn failed_jobs_can_be_queued_again_and_cancelled_jobs_stay_cancelled() {
        let (store, admin) = store_with_admin();
        let id = store
            .enqueue(&new_job(&episode(1), admin.id))
            .unwrap()
            .unwrap();
        store.claim_next().unwrap();
        store.fail(id, "no hoster answered").unwrap();
        assert_eq!(
            store.jobs().unwrap()[0].error.as_deref(),
            Some("no hoster answered")
        );

        assert_eq!(
            store.enqueue(&new_job(&episode(1), admin.id)).unwrap(),
            Some(id)
        );
        store.claim_next().unwrap();
        assert!(store.cancel(id).unwrap());
        store
            .finish(id, "/downloads/x.mkv", Language::GermanSub)
            .unwrap();

        let job = &store.jobs().unwrap()[0];
        assert_eq!(
            (job.state, job.path.as_deref()),
            (JobState::Cancelled, None)
        );
    }

    #[test]
    fn accounts_can_be_renamed_and_keep_email_and_theme() {
        let (store, admin) = store_with_admin();
        let other = store.create_user("sam", "h", Role::User).unwrap().unwrap();
        let account = store.account(admin.id).unwrap().unwrap();
        assert_eq!((account.email, account.theme), (None, Theme::default()));

        // Names compare without case.
        assert!(!store.rename_user(admin.id, "SAM").unwrap());
        assert!(store.rename_user(admin.id, "Boss").unwrap());
        // Only the case changes.
        assert!(store.rename_user(other.id, "Sam").unwrap());

        assert!(
            store
                .set_email(admin.id, Some("boss@example.org"), 0)
                .unwrap()
        );
        let theme = Theme {
            music: false,
            volume: 30,
            ascii: true,
            ascii_piece: "tokyo-rain".into(),
            ..Theme::default()
        };
        store.set_theme(admin.id, &theme).unwrap();
        let account = store.account(admin.id).unwrap().unwrap();
        assert_eq!(account.name, "Boss");
        assert_eq!(account.email.as_deref(), Some("boss@example.org"));
        assert_eq!(account.theme, theme);
        assert!(store.credentials("boss").unwrap().is_some());
    }

    #[test]
    fn api_keys_belong_to_one_user_and_die_with_them() {
        let (store, admin) = store_with_admin();
        let sam = store.create_user("sam", "h", Role::User).unwrap().unwrap();
        let key = store.create_api_key(sam.id, "Musubi", "hash", 10).unwrap();
        assert_eq!(store.api_key_user("hash", 20).unwrap(), Some(sam.clone()));
        assert_eq!(store.api_keys(sam.id).unwrap()[0].last_used_at, Some(20));
        assert_eq!(store.api_key_user("other", 20).unwrap(), None);
        // Another user cannot delete it.
        assert!(!store.delete_api_key(key.id, admin.id).unwrap());
        assert!(store.delete_user(sam.id).unwrap());
        assert_eq!(store.api_key_user("hash", 30).unwrap(), None);
        assert!(store.api_keys(sam.id).unwrap().is_empty());
    }

    #[test]
    fn settings_from_before_the_backdrop_choice_use_glass() {
        let old: Settings = serde_json::from_str(r#"{"parallel": 3}"#).unwrap();
        assert_eq!(old.backdrop, kura_domain::Backdrop::Glass);
    }

    #[test]
    fn done_jobs_are_queued_again_only_when_asked() {
        let (store, admin) = store_with_admin();
        let first = episode(1);
        let new = new_job(&first, admin.id);
        let id = store.enqueue(&new).unwrap().unwrap();
        assert_eq!(store.jobs().unwrap()[0].started_at, None);
        store.claim_next().unwrap();
        assert!(store.jobs().unwrap()[0].started_at.is_some());
        store
            .finish(id, "/media/x.mkv", Language::GermanSub)
            .unwrap();

        assert_eq!(store.enqueue(&new).unwrap(), None);
        let again = NewJob {
            replace_done: true,
            replaces: &[],
            ..new_job(&first, admin.id)
        };
        assert_eq!(store.enqueue(&again).unwrap(), Some(id));
        let job = &store.jobs().unwrap()[0];
        assert_eq!(job.state, JobState::Queued);
        // Started once, so Auto-Sync keeps following the title.
        assert!(job.started_at.is_some());
        // A queued or running job is never replaced.
        assert_eq!(store.enqueue(&again).unwrap(), None);
    }

    #[test]
    fn cleared_downloads_stay_known_and_keep_their_choices() {
        let (store, admin) = store_with_admin();
        let id = store.enqueue(&new_job(&episode(1), admin.id)).unwrap();
        store.enqueue(&new_job(&episode(2), admin.id)).unwrap();
        store.claim_next().unwrap();
        store
            .finish(id.unwrap(), "/media/x.mkv", Language::GermanSub)
            .unwrap();

        // Only the finished one is cleared; the waiting one stays visible.
        assert_eq!(store.hide_finished().unwrap(), 1);
        let jobs = store.jobs().unwrap();
        assert_eq!((jobs[0].hidden, jobs[1].hidden), (true, false));
        assert_eq!(jobs[0].language, Some(Language::GermanSub));
        assert_eq!((jobs[0].dir.as_str(), jobs[0].queued_at), ("/media", 100));

        // A cleared download is still done, so it is not queued again.
        assert_eq!(
            store.enqueue(&new_job(&episode(1), admin.id)).unwrap(),
            None
        );
    }

    #[test]
    fn requests_are_listed_per_user_and_decided_once() {
        let (store, admin) = store_with_admin();
        let sam = store.create_user("sam", "x", Role::User).unwrap().unwrap();
        let picked = Picked {
            all: false,
            seasons: vec![2],
            episodes: vec![(1, 3)],
        };
        let new = |user_id| NewRequest {
            user_id,
            site: SiteId::AniWorld,
            slug: "frieren",
            series_title: "Frieren",
            picked: &picked,
            language: Some(Language::GermanSub),
            hoster: Some("voe"),
            imdb_id: Some("none"),
            now: 10,
        };
        let first = store.create_request(&new(sam.id)).unwrap();
        let second = store.create_request(&new(admin.id)).unwrap();

        let own = store.requests(Some(sam.id)).unwrap();
        assert_eq!(own.len(), 1);
        assert_eq!((own[0].id, own[0].user_name.as_str()), (first, "sam"));
        assert_eq!(own[0].picked, picked);
        assert_eq!(own[0].language, Some(Language::GermanSub));

        let decided = store
            .decide_request(first, RequestState::Accepted, 20)
            .unwrap();
        assert_eq!(decided.unwrap().state, RequestState::Accepted);
        let again = store
            .decide_request(first, RequestState::Rejected, 30)
            .unwrap();
        assert!(again.is_none());

        // Open ones come first.
        let all: Vec<_> = store.requests(None).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(all, [second, first]);

        store.reopen_request(first).unwrap();
        assert_eq!(
            store.requests(Some(sam.id)).unwrap()[0].state,
            RequestState::Open
        );

        // Only the owner can cancel, and only while it is open.
        assert!(!store.cancel_request(first, admin.id).unwrap());
        assert!(store.cancel_request(first, sam.id).unwrap());
        assert!(!store.cancel_request(first, sam.id).unwrap());
        let third = store.create_request(&new(sam.id)).unwrap();
        store
            .decide_request(third, RequestState::Rejected, 40)
            .unwrap();
        assert!(!store.cancel_request(third, sam.id).unwrap());
        // Clearing removes decided requests and keeps open ones.
        assert_eq!(store.clear_decided_requests().unwrap(), 1);
        let left: Vec<_> = store.requests(None).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(left, [second]);
        store.create_request(&new(sam.id)).unwrap();

        // A deleted user's requests go with them.
        store.delete_user(sam.id).unwrap();
        assert_eq!(store.requests(None).unwrap().len(), 1);
    }

    #[test]
    fn a_new_password_ends_the_other_sessions() {
        let (store, admin) = store_with_admin();
        store.create_session("here", admin.id, 100, 1000).unwrap();
        store.create_session("there", admin.id, 100, 1000).unwrap();
        store.set_password(admin.id, "new-hash", "here").unwrap();
        assert_eq!(store.credentials("admin").unwrap().unwrap().1, "new-hash");
        assert!(store.session_user("here", 101).unwrap().is_some());
        assert!(store.session_user("there", 101).unwrap().is_none());
    }

    #[test]
    fn a_database_from_before_requests_keeps_its_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kura.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            Migrations::new(vec![M::up(SCHEMA)])
                .to_latest(&mut conn)
                .unwrap();
            conn.execute(
                "INSERT INTO jobs (site, slug, season, episode, series_title, state, dir, queued_at)
                 VALUES ('aniworld', 'frieren', 1, 1, 'Frieren', 'done', '/media', 5)",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let jobs = store.jobs().unwrap();
        assert_eq!((jobs.len(), jobs[0].imdb_id.as_deref()), (1, None));
        assert!(store.requests(None).unwrap().is_empty());
    }

    #[test]
    fn interrupted_jobs_start_over_after_a_restart() {
        let (store, admin) = store_with_admin();
        let id = store
            .enqueue(&new_job(&episode(1), admin.id))
            .unwrap()
            .unwrap();
        store.claim_next().unwrap();
        store.set_progress(id, 0.5).unwrap();
        assert_eq!(store.jobs().unwrap()[0].progress, 0.5);

        store.requeue_interrupted().unwrap();
        let job = store.claim_next().unwrap().unwrap();
        assert_eq!((job.id, job.progress), (id, 0.0));
    }

    #[test]
    fn a_reset_link_works_once_and_until_it_expires() {
        let (store, admin) = store_with_admin();
        store.create_reset("one", admin.id, 100, 60).unwrap();
        assert_eq!(store.last_reset(admin.id, 100).unwrap(), Some(100));
        // Expired at 160.
        assert_eq!(store.take_reset("one", 160).unwrap(), None);
        assert_eq!(store.last_reset(admin.id, 160).unwrap(), None);

        store.create_reset("two", admin.id, 200, 60).unwrap();
        assert_eq!(store.take_reset("two", 259).unwrap(), Some(admin.id));
        assert_eq!(store.take_reset("two", 259).unwrap(), None);

        // A newer link replaces an older one.
        store.create_reset("old", admin.id, 300, 60).unwrap();
        store.create_reset("new", admin.id, 301, 60).unwrap();
        assert_eq!(store.take_reset("old", 302).unwrap(), None);
        assert_eq!(store.take_reset("new", 302).unwrap(), Some(admin.id));
    }

    #[test]
    fn a_new_password_or_email_ends_the_reset_links() {
        let (store, admin) = store_with_admin();
        store.create_reset("one", admin.id, 100, 60).unwrap();
        store.set_password(admin.id, "new-hash", "").unwrap();
        assert_eq!(store.take_reset("one", 101).unwrap(), None);

        store.create_reset("two", admin.id, 100, 60).unwrap();
        store
            .set_email(admin.id, Some("admin@example.org"), 0)
            .unwrap();
        assert_eq!(store.take_reset("two", 101).unwrap(), None);

        // Deleting the user deletes their links.
        let sam = store
            .create_user("sam", "hash", Role::User)
            .unwrap()
            .unwrap();
        store.create_reset("three", sam.id, 100, 60).unwrap();
        assert!(store.delete_user(sam.id).unwrap());
        assert_eq!(store.take_reset("three", 101).unwrap(), None);
    }

    #[test]
    fn resets_find_users_by_name_or_email() {
        let (store, admin) = store_with_admin();
        store
            .set_email(admin.id, Some("Admin@Example.org"), 0)
            .unwrap();
        let found = |text| {
            store
                .users_named(text)
                .unwrap()
                .into_iter()
                .map(|user| user.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(found("ADMIN"), vec![admin.id]);
        assert_eq!(found("admin@example.ORG"), vec![admin.id]);
        assert!(found("nobody").is_empty());
        assert!(found("%").is_empty());
    }

    #[test]
    fn the_mail_settings_are_kept_apart_from_the_settings() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.mail().unwrap(), Mail::default());
        let mail = Mail {
            host: "smtp.example.org".into(),
            port: 587,
            password: "secret".into(),
            ..Mail::default()
        };
        store.save_mail(&mail).unwrap();
        assert_eq!(store.mail().unwrap(), mail);
        let settings = serde_json::to_string(&store.settings().unwrap()).unwrap();
        assert!(!settings.contains("secret"));
    }

    #[test]
    fn an_invite_works_once_until_it_expires_and_ends_with_its_admin() {
        let (store, admin) = store_with_admin();
        store
            .create_invite("one", "kim@example.org", Role::User, admin.id, 100, 60)
            .unwrap();
        // Expired at 160.
        assert_eq!(
            store.redeem_invite("one", "kim", "hash", 160).unwrap(),
            Redeemed::Invalid
        );
        assert!(store.invites(160).unwrap().is_empty());

        store
            .create_invite("two", "kim@example.org", Role::Admin, admin.id, 200, 60)
            .unwrap();
        // A newer invite for the same address replaces it.
        store
            .create_invite("three", "KIM@example.org", Role::User, admin.id, 201, 60)
            .unwrap();
        assert_eq!(
            store.redeem_invite("two", "kim", "hash", 202).unwrap(),
            Redeemed::Invalid
        );
        assert_eq!(
            store.redeem_invite("three", "admin", "hash", 202).unwrap(),
            Redeemed::NameTaken
        );
        let Redeemed::Created(kim) = store.redeem_invite("three", "kim", "hash", 202).unwrap()
        else {
            panic!("the invite should work");
        };
        assert_eq!(kim.role, Role::User);
        assert_eq!(
            store.account(kim.id).unwrap().unwrap().email.as_deref(),
            Some("KIM@example.org")
        );
        assert_eq!(
            store.redeem_invite("three", "kim2", "hash", 203).unwrap(),
            Redeemed::Invalid
        );

        // Deleting the admin who sent an invite ends it.
        let other = store
            .create_user("other", "hash", Role::Admin)
            .unwrap()
            .unwrap();
        store
            .create_invite("four", "lee@example.org", Role::User, other.id, 300, 60)
            .unwrap();
        assert!(store.delete_user(other.id).unwrap());
        assert_eq!(
            store.redeem_invite("four", "lee", "hash", 301).unwrap(),
            Redeemed::Invalid
        );
    }

    #[test]
    fn an_email_change_waits_for_its_link_and_every_address_is_one_account_s() {
        let (store, admin) = store_with_admin();
        let sam = store
            .create_user("sam", "hash", Role::User)
            .unwrap()
            .unwrap();
        assert!(
            store
                .set_email(admin.id, Some("Ärger@Example.org"), 100)
                .unwrap()
        );
        // Compared without regard to case, also beyond A to Z.
        assert!(
            !store
                .set_email(sam.id, Some("ärger@example.ORG"), 100)
                .unwrap()
        );
        assert!(store.email_taken("ÄRGER@example.org", None, 100).unwrap());
        assert!(
            !store
                .email_taken("ÄRGER@example.org", Some(admin.id), 100)
                .unwrap()
        );
        assert!(
            !store
                .start_email_change(sam.id, "ärger@example.org", "t0", 100, 50)
                .unwrap()
        );

        // A change counts only once its link is used, and only before it expires.
        assert!(
            store
                .start_email_change(sam.id, "sam@example.org", "t1", 100, 50)
                .unwrap()
        );
        assert_eq!(store.account(sam.id).unwrap().unwrap().email, None);
        assert_eq!(store.users_named("sam@example.org").unwrap().len(), 0);
        assert_eq!(
            store.confirm_email_change("t1", 150).unwrap(),
            EmailConfirmed::Invalid
        );
        assert!(
            store
                .start_email_change(sam.id, "sam@example.org", "t2", 200, 50)
                .unwrap()
        );
        store.create_reset("reset", sam.id, 200, 100).unwrap();
        assert_eq!(
            store.confirm_email_change("t2", 210).unwrap(),
            EmailConfirmed::Changed {
                user_id: sam.id,
                email: "sam@example.org".to_owned()
            }
        );
        assert_eq!(
            store.confirm_email_change("t2", 211).unwrap(),
            EmailConfirmed::Invalid
        );
        // The reset link sent before went elsewhere; it ends.
        assert_eq!(store.take_reset("reset", 212).unwrap(), None);

        // An address taken while its link waits is not given away twice.
        assert!(
            store
                .start_email_change(sam.id, "new@example.org", "t3", 300, 50)
                .unwrap()
        );
        assert!(
            store
                .set_email(admin.id, Some("NEW@example.org"), 301)
                .unwrap()
        );
        assert_eq!(
            store.confirm_email_change("t3", 302).unwrap(),
            EmailConfirmed::Taken
        );
        assert_eq!(
            store.account(sam.id).unwrap().unwrap().email.as_deref(),
            Some("sam@example.org")
        );

        // An invite whose address an account took meanwhile creates no second one.
        store
            .create_invite("inv", "kim@example.org", Role::User, admin.id, 400, 100)
            .unwrap();
        assert!(
            !store
                .set_email(sam.id, Some("KIM@example.org"), 401)
                .unwrap()
        );
        store
            .conn()
            .execute(
                "UPDATE users SET email = 'Kim@Example.org' WHERE id = ?1",
                [sam.id],
            )
            .unwrap();
        assert_eq!(
            store.redeem_invite("inv", "kim", "hash", 402).unwrap(),
            Redeemed::EmailTaken
        );
    }
}
