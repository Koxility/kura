//! Jellyfin: once the queue is empty, Kura asks Jellyfin to scan the series folders that got
//! new episodes, and only those. One scan per emptied queue, however many downloads it held,
//! so Jellyfin is not asked again and again while a season comes in.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError},
    time::Duration,
};

use kura_domain::{Jellyfin, Library};
use serde::{Deserialize, Serialize};

use crate::{Core, CoreError, Result, downloaded};

/// Series folders with new episodes since the last scan.
pub(crate) type Pending = BTreeSet<PathBuf>;

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct MediaUpdated {
    updates: Vec<MediaUpdate>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct MediaUpdate {
    path: String,
    update_type: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SystemInfo {
    #[serde(default)]
    server_name: String,
    #[serde(default)]
    version: String,
}

impl Core {
    /// Remembers the series folder of a finished download for the next scan.
    pub(crate) fn note_new_file(&self, file: &Path) {
        if let Some(folder) = downloaded::series_folder(file) {
            self.pending_scan().insert(folder);
        }
    }

    /// Called when nothing is queued or running any more: tells Jellyfin about the series
    /// folders that got new episodes. Failed downloads stay in the queue and do not hold
    /// back the scan of what did arrive.
    pub(crate) fn queue_drained(self: &Arc<Self>) {
        let folders = std::mem::take(&mut *self.pending_scan());
        if folders.is_empty() {
            return;
        }
        let settings = match self.store.settings() {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!("could not read the settings for the Jellyfin scan: {err}");
                return;
            }
        };
        if !settings.jellyfin.enabled() {
            return;
        }
        let paths: Vec<String> = folders
            .iter()
            .map(|folder| jellyfin_path(&settings.libraries, folder))
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        tokio::spawn(async move {
            match refresh(&settings.jellyfin, &paths).await {
                Ok(()) => tracing::info!("asked Jellyfin to scan {}", paths.join(", ")),
                Err(err) => tracing::warn!("could not ask Jellyfin to scan: {err}"),
            }
        });
    }

    /// Checks that Jellyfin answers with these settings; returns its name and version.
    pub async fn test_jellyfin(&self, jellyfin: &Jellyfin) -> Result<String> {
        let mut jellyfin = jellyfin.clone();
        self.keep_jellyfin_key(&mut jellyfin)?;
        let jellyfin = &jellyfin;
        check(jellyfin)?;
        let answer = client()?
            .get(format!("{}/System/Info", base(jellyfin)))
            .header("Authorization", authorization(jellyfin))
            .send()
            .await
            .map_err(failed)?;
        let info: SystemInfo = answer
            .error_for_status()
            .map_err(failed)?
            .json()
            .await
            .map_err(|_| CoreError::Jellyfin("the server did not answer like Jellyfin".into()))?;
        Ok(format!("{} {}", info.server_name, info.version)
            .trim()
            .to_owned())
    }

    /// The browser never gets the stored API key, so it sends an empty one to mean "keep it".
    /// That only holds for the very same server: otherwise the stored key would go to whatever
    /// address is entered, and an admin, or a key that got out, could read it that way.
    pub(crate) fn keep_jellyfin_key(&self, jellyfin: &mut Jellyfin) -> Result<()> {
        if !jellyfin.api_key.trim().is_empty() || !jellyfin.enabled() {
            return Ok(());
        }
        let stored = self.store.settings()?.jellyfin;
        if stored.enabled() && same_server(&stored.url, &jellyfin.url) {
            jellyfin.api_key = stored.api_key;
        }
        Ok(())
    }

    fn pending_scan(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending_scan
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Refuses an address that is not HTTP(S) and an address without a key.
pub(crate) fn check(jellyfin: &Jellyfin) -> Result<()> {
    if !jellyfin.enabled() {
        return Err(CoreError::Invalid(
            "enter the address of the Jellyfin server",
        ));
    }
    let url = jellyfin.url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(CoreError::Invalid(
            "the Jellyfin address must start with http:// or https://",
        ));
    }
    if jellyfin.api_key.trim().is_empty() {
        return Err(CoreError::Invalid("Jellyfin needs an API key"));
    }
    Ok(())
}

/// Whether two Jellyfin addresses name the same server, as people type them.
fn same_server(a: &str, b: &str) -> bool {
    let plain = |url: &str| url.trim().trim_end_matches('/').to_ascii_lowercase();
    plain(a) == plain(b)
}

/// The folder as Jellyfin sees it: the library's Jellyfin path in place of its own, when the
/// library has one. The innermost library wins, in case one lies inside another.
pub(crate) fn jellyfin_path(libraries: &[Library], folder: &Path) -> PathBuf {
    let library = libraries
        .iter()
        .filter_map(|library| Some((library, folder.strip_prefix(&library.path).ok()?)))
        .max_by_key(|(library, _)| library.path.components().count());
    match library {
        Some((library, rest)) => library
            .jellyfin_path
            .as_ref()
            .map_or_else(|| folder.to_owned(), |mapped| mapped.join(rest)),
        None => folder.to_owned(),
    }
}

/// One request for all folders. Jellyfin scans just these paths, not its whole library.
async fn refresh(jellyfin: &Jellyfin, paths: &[String]) -> Result<()> {
    let body = MediaUpdated {
        updates: paths
            .iter()
            .map(|path| MediaUpdate {
                path: path.clone(),
                update_type: "Modified",
            })
            .collect(),
    };
    client()?
        .post(format!("{}/Library/Media/Updated", base(jellyfin)))
        .header("Authorization", authorization(jellyfin))
        .json(&body)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(failed)?;
    Ok(())
}

/// A plain client: the admin names the server, and it is usually in the home network, which
/// the guard for sites and hosters would refuse.
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn base(jellyfin: &Jellyfin) -> &str {
    jellyfin.url.trim().trim_end_matches('/')
}

fn authorization(jellyfin: &Jellyfin) -> String {
    format!(
        "MediaBrowser Client=\"Kura\", Version=\"{}\", Token=\"{}\"",
        env!("CARGO_PKG_VERSION"),
        jellyfin.api_key.trim()
    )
}

/// What went wrong, in fixed words: what a server at the address wrote is not passed on, so
/// the test cannot read out other services in Kura's network. The whole error goes to the log.
fn failed(err: reqwest::Error) -> CoreError {
    tracing::warn!("Jellyfin: {err}");
    let text = match err.status() {
        Some(status) if status.as_u16() == 401 => "Jellyfin refused the API key".to_owned(),
        Some(status) => format!("Jellyfin answered {status}"),
        None if err.is_timeout() => "Jellyfin did not answer in time".to_owned(),
        None => "could not reach Jellyfin; check the address (details in Kura's log)".to_owned(),
    };
    CoreError::Jellyfin(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(path: &str, jellyfin: Option<&str>) -> Library {
        Library {
            name: path.to_owned(),
            path: path.into(),
            default_for: Vec::new(),
            jellyfin_path: jellyfin.map(PathBuf::from),
        }
    }

    #[test]
    fn folders_are_named_as_jellyfin_sees_them() {
        let libraries = [
            library("/media", Some("/data/tv")),
            library("/media/anime", Some("/data/anime")),
            library("/other", None),
            library("/media/plain", None),
        ];
        let mapped = |folder: &str| jellyfin_path(&libraries, Path::new(folder));
        assert_eq!(
            mapped("/media/Frieren [imdbid-tt1]"),
            Path::new("/data/tv/Frieren [imdbid-tt1]")
        );
        assert_eq!(
            mapped("/media/anime/One Piece"),
            Path::new("/data/anime/One Piece")
        );
        assert_eq!(mapped("/other/Dark"), Path::new("/other/Dark"));
        assert_eq!(mapped("/media/plain/Dark"), Path::new("/media/plain/Dark"));
        assert_eq!(mapped("/elsewhere/Dark"), Path::new("/elsewhere/Dark"));
    }

    #[test]
    fn settings_need_an_http_address_and_a_key() {
        let jellyfin = |url: &str, api_key: &str| Jellyfin {
            url: url.to_owned(),
            api_key: api_key.to_owned(),
        };
        assert!(check(&jellyfin("http://jellyfin:8096", "key")).is_ok());
        assert!(check(&jellyfin("jellyfin:8096", "key")).is_err());
        assert!(check(&jellyfin("https://jellyfin", " ")).is_err());
        assert!(check(&jellyfin("", "key")).is_err());
    }
}
