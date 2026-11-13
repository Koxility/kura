//! SQLite persistence: users, sessions, settings and the download queue.
//!
//! Finished jobs keep the path of their file, so the queue table is also the library index.

use std::{
    path::Path,
    str::FromStr,
    sync::{Mutex, MutexGuard, PoisonError},
};

use kura_domain::{EpisodeId, Job, Language, Picked, Request, RequestState, Role, Settings, User};
use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
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

const JOB_COLUMNS: &str = "id, site, slug, season, episode, series_title, state, progress, error, path, \
     language, hoster, dir, hidden, queued_at, imdb_id";

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
        Migrations::new(vec![M::up(SCHEMA), M::up(REQUESTS_AND_IMDB)]).to_latest(&mut conn)?;
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

    /// Replaces a user's password hash and ends their other sessions, so a changed password
    /// also logs out every other device.
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
        Ok(())
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
    /// episode is queued again with the new choices; `None` means it is already queued,
    /// running or done.
    pub fn enqueue(&self, new: &NewJob) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "INSERT INTO jobs (site, slug, season, episode, series_title, user_id,
                                   language, hoster, dir, queued_at, imdb_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (site, slug, season, episode) DO UPDATE
                     SET state = 'queued', progress = 0, error = NULL, hidden = 0,
                         user_id = excluded.user_id, language = excluded.language,
                         hoster = excluded.hoster, dir = excluded.dir,
                         queued_at = excluded.queued_at, imdb_id = excluded.imdb_id
                     WHERE state IN ('failed', 'cancelled')
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
                    new.imdb_id
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

    /// Marks the oldest queued job as running and returns it.
    pub fn claim_next(&self) -> Result<Option<Job>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "UPDATE jobs SET state = 'running'
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

    pub fn finish(&self, id: i64, path: &str) -> Result<()> {
        self.update_running("state = 'done', progress = 1, path = ?2", params![id, path])
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
        store.finish(id, "/downloads/x.mkv").unwrap();

        let job = &store.jobs().unwrap()[0];
        assert_eq!(
            (job.state, job.path.as_deref()),
            (JobState::Cancelled, None)
        );
    }

    #[test]
    fn cleared_downloads_stay_known_and_keep_their_choices() {
        let (store, admin) = store_with_admin();
        let id = store.enqueue(&new_job(&episode(1), admin.id)).unwrap();
        store.enqueue(&new_job(&episode(2), admin.id)).unwrap();
        store.claim_next().unwrap();
        store.finish(id.unwrap(), "/media/x.mkv").unwrap();

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
}
