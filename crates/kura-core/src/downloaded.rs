//! What is already downloaded: finished downloads in the database, and series folders found
//! in the libraries, matched by IMDb ID or by name. Folders count even when Kura did not
//! create them, so a library filled by hand or by another tool is recognised too. File names
//! also tell the language an episode is in.
//!
//! Nothing here is trusted for long: a finished download counts only while its file is still
//! there, a folder only while it holds episodes, and the libraries are read again every few
//! minutes, before every Auto-Sync run and whenever a title's page is opened.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};

use kura_domain::{Job, JobState, Language, SeriesSummary, SiteId};

use crate::{Core, Result, naming::language_in};

/// Library folders are read at most this often for lists; pages ask for every list they
/// show. A title's page and Auto-Sync always read them again.
const SCAN_EVERY: Duration = Duration::from_secs(5 * 60);

/// How often the background check looks for files that were added or deleted.
pub const RESCAN_EVERY: Duration = Duration::from_secs(5 * 60);

const VIDEO: [&str; 6] = ["mkv", "mp4", "m4v", "avi", "webm", "ts"];

/// What Jellyfin keeps beside an episode file, by the file's name without its extension.
const BESIDE: [&str; 5] = [
    ".nfo",
    "-thumb.jpg",
    "-thumb.jpeg",
    "-thumb.png",
    "-thumb.webp",
];

/// A series folder in a library.
#[derive(Debug, Clone)]
pub(crate) struct TitleFolder {
    path: PathBuf,
    key: String,
    imdb_id: Option<String>,
    /// Whether any episode file is in it; an emptied folder is not a download.
    has_episodes: bool,
}

/// What one read of the libraries found.
pub(crate) struct Scan {
    at: Instant,
    folders: Arc<Vec<TitleFolder>>,
    /// Series with a finished download whose file is still there.
    on_disk: Arc<HashSet<(SiteId, String)>>,
}

/// Series a title page found downloaded episodes for, with the title and IMDb ID it was
/// checked with, so the background check can check it again.
pub(crate) type FoundTitles = HashMap<(SiteId, String), (String, Option<String>)>;

/// An episode file in a library: its `(season, episode)`, and its language when the record of
/// the download or the file name tells it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Found {
    pub episode: (u32, u32),
    pub language: Option<Language>,
    pub path: PathBuf,
}

impl Core {
    /// Sets `downloaded` on every series of which at least one episode is downloaded. Lists
    /// carry no IMDb ID, so series found by ID on their own page count from then on.
    pub(crate) fn mark_downloaded(&self, list: &mut [SeriesSummary]) -> Result<()> {
        let (folders, on_disk) = self.scanned()?;
        let keys: HashSet<&str> = folders
            .iter()
            .filter(|folder| folder.has_episodes)
            .map(|folder| folder.key.as_str())
            .collect();
        let found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        for series in list {
            let id = (series.site, series.slug.clone());
            series.downloaded = on_disk.contains(&id)
                || keys.contains(title_key(&series.title).as_str())
                || found.contains_key(&id);
        }
        Ok(())
    }

    /// `(season, episode)` pairs of a series that are downloaded, sorted. Finished downloads
    /// count, and episode files in a library folder that carries the series' IMDb ID or name.
    pub fn downloaded_episodes(
        &self,
        site: SiteId,
        slug: &str,
        title: &str,
        imdb_id: Option<&str>,
    ) -> Result<Vec<(u32, u32)>> {
        let found = self.downloaded_with_language(site, slug, title, imdb_id)?;
        let mut pairs: Vec<_> = found.into_iter().map(|file| file.episode).collect();
        pairs.sort_unstable();
        pairs.dedup();
        Ok(pairs)
    }

    /// Every episode file behind [`Self::downloaded_episodes`], with its language where known.
    /// An episode can have more than one file, e.g. in two languages.
    pub(crate) fn downloaded_with_language(
        &self,
        site: SiteId,
        slug: &str,
        title: &str,
        imdb_id: Option<&str>,
    ) -> Result<Vec<Found>> {
        let jobs = self.store.jobs()?;
        let ours = jobs.iter().filter(|job| {
            job.state == JobState::Done && job.episode.site == site && job.episode.slug == slug
        });
        let mut found: Vec<Found> = Vec::new();
        let mut folders: Vec<PathBuf> = Vec::new();
        for job in ours {
            let path = job.path.as_deref().map(Path::new);
            // The folder a download went to, even if it matches neither ID nor name.
            if let Some(folder) = path.and_then(series_folder) {
                folders.push(folder);
            }
            // A file deleted since is no longer downloaded.
            if !on_disk(job) {
                continue;
            }
            let Some(path) = path else { continue };
            // Downloads from before 1.2.2 recorded only the picked language; the name has it.
            let named = path
                .file_name()
                .and_then(|name| language_in(&name.to_string_lossy()));
            found.push(Found {
                episode: (job.episode.season, job.episode.number),
                language: job.language.or(named),
                path: path.to_owned(),
            });
        }

        let key = title_key(title);
        let imdb_id = imdb_id.filter(|id| *id != "none");
        for folder in self.scanned()?.0.iter() {
            let same_id = imdb_id.is_some() && folder.imdb_id.as_deref() == imdb_id;
            // A folder that names another IMDb ID is another series, whatever its name.
            let other_id = imdb_id.is_some() && folder.imdb_id.is_some() && !same_id;
            if same_id || (folder.key == key && !other_id) {
                folders.push(folder.path.clone());
            }
        }
        folders.sort();
        folders.dedup();
        for folder in &folders {
            episodes_in(folder, 0, &mut found);
        }
        // One entry per file: a download's own record comes first and knows the language best.
        let mut seen = HashSet::new();
        found.retain(|file| seen.insert(file.path.clone()));
        found.sort_by(|a, b| a.episode.cmp(&b.episode).then_with(|| a.path.cmp(&b.path)));
        let mut known = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        if found.is_empty() {
            known.remove(&(site, slug.to_owned()));
        } else {
            let names = (title.to_owned(), imdb_id.map(str::to_owned));
            known.insert((site, slug.to_owned()), names);
        }
        Ok(found)
    }

    /// Reads the libraries again and checks every title found before, so files deleted
    /// since are no longer marked. Runs every few minutes; reads files, so off the runtime.
    pub fn refresh_downloaded(&self) -> Result<()> {
        self.forget_scan();
        self.scanned()?;
        let known: Vec<_> = {
            let found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
            found
                .iter()
                .map(|(id, names)| (id.clone(), names.clone()))
                .collect()
        };
        for ((site, slug), (title, imdb_id)) in known {
            self.downloaded_with_language(site, &slug, &title, imdb_id.as_deref())?;
        }
        Ok(())
    }

    /// Calls [`Self::refresh_downloaded`] every few minutes, for the life of the server.
    pub async fn run_library_scan(self: Arc<Self>) {
        loop {
            tokio::time::sleep(RESCAN_EVERY).await;
            let core = self.clone();
            match tokio::task::spawn_blocking(move || core.refresh_downloaded()).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::warn!("could not check the libraries: {err}"),
                Err(err) => tracing::warn!("the library check stopped: {err}"),
            }
        }
    }

    /// Series folders directly in every library, and in a `Series` folder inside it where
    /// Kura 1.0 and 1.1 put them; and the series whose finished downloads are still there.
    #[allow(clippy::type_complexity)]
    fn scanned(&self) -> Result<(Arc<Vec<TitleFolder>>, Arc<HashSet<(SiteId, String)>>)> {
        let lock = || self.scan.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(scan) = lock().as_ref()
            && scan.at.elapsed() < SCAN_EVERY
        {
            return Ok((scan.folders.clone(), scan.on_disk.clone()));
        }
        let mut folders = Vec::new();
        for library in self.store.settings()?.libraries {
            for dir in [library.path.clone(), library.path.join("Series")] {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                    // `Series` itself is a container, not a title.
                    if !is_dir || name.starts_with('.') || name == "Series" {
                        continue;
                    }
                    let mut episodes = Vec::new();
                    episodes_in(&entry.path(), 0, &mut episodes);
                    folders.push(TitleFolder {
                        key: title_key(&name),
                        imdb_id: folder_imdb_id(&name),
                        path: entry.path(),
                        has_episodes: !episodes.is_empty(),
                    });
                }
            }
        }
        let jobs = self.store.jobs()?;
        let on_disk: HashSet<_> = jobs
            .iter()
            .filter(|job| job.state == JobState::Done && on_disk(job))
            .map(|job| (job.episode.site, job.episode.slug.clone()))
            .collect();
        let folders = Arc::new(folders);
        let on_disk = Arc::new(on_disk);
        *lock() = Some(Scan {
            at: Instant::now(),
            folders: folders.clone(),
            on_disk: on_disk.clone(),
        });
        Ok((folders, on_disk))
    }

    /// Forgets the last look at the libraries if it is at least `age` old, so the next
    /// question reads them again; a younger one is kept.
    pub fn forget_scan_older_than(&self, age: Duration) {
        let mut scan = self.scan.lock().unwrap_or_else(PoisonError::into_inner);
        if scan.as_ref().is_some_and(|scan| scan.at.elapsed() >= age) {
            *scan = None;
        }
    }

    /// Deletes the files a finished download replaces, each with the NFO and thumbnail
    /// Jellyfin keeps beside it, so Jellyfin reads the episode afresh. Only files inside a
    /// library are touched; one that cannot be deleted is logged and stays.
    pub(crate) fn remove_replaced(&self, job: &Job, new: &Path) {
        if job.replaces.is_empty() {
            return;
        }
        let libraries = match self.store.settings() {
            Ok(settings) => settings.libraries,
            Err(err) => {
                tracing::warn!("could not read the libraries to replace old files: {err}");
                return;
            }
        };
        let roots: Vec<PathBuf> = libraries
            .iter()
            .filter_map(|library| library.path.canonicalize().ok())
            .collect();
        let inside = |file: &Path| {
            let file = file.canonicalize();
            file.is_ok_and(|file| roots.iter().any(|root| file.starts_with(root)))
        };
        for old in job.replaces.iter().map(Path::new) {
            let Some(stem) = old.file_stem() else {
                continue;
            };
            // A name without the language is written over, not deleted.
            let video = (old != new).then(|| old.to_owned());
            let beside = BESIDE.iter().map(|end| {
                let mut name = stem.to_os_string();
                name.push(end);
                old.with_file_name(name)
            });
            for file in video.into_iter().chain(beside) {
                if !file.is_file() || !inside(&file) {
                    continue;
                }
                match std::fs::remove_file(&file) {
                    Ok(()) => {
                        tracing::info!("removed {}, replaced by {}", file.display(), new.display())
                    }
                    Err(err) => tracing::warn!("could not remove {}: {err}", file.display()),
                }
            }
        }
    }

    /// Makes the next lookup read the library folders again, e.g. after a download.
    pub fn forget_scan(&self) {
        *self.scan.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Whether the file of a finished download is still there.
pub(crate) fn on_disk(job: &Job) -> bool {
    job.path
        .as_deref()
        .is_some_and(|path| Path::new(path).is_file())
}

/// The series folder of an episode file: `<series>/Season 01/<file>`.
pub(crate) fn series_folder(file: &Path) -> Option<PathBuf> {
    Some(file.parent()?.parent()?.to_owned())
}

/// A name reduced to lower-case letters and digits, without spaces and without what is in
/// brackets, so `One Piece [imdbid-tt0388629]`, `OnePiece`, `one-piece!` and
/// `One Piece (1999)` are the same title.
pub(crate) fn title_key(name: &str) -> String {
    let letters = |outside_brackets: bool| {
        let mut key = String::new();
        let mut depth = 0u32;
        for c in name.chars() {
            match c {
                '(' | '[' if outside_brackets => depth += 1,
                ')' | ']' if outside_brackets => depth = depth.saturating_sub(1),
                c if depth == 0 && c.is_alphanumeric() => key.extend(c.to_lowercase()),
                _ => {}
            }
        }
        key
    };
    let key = letters(true);
    // A name that is all brackets keeps them, so it does not match every other such name.
    if key.is_empty() { letters(false) } else { key }
}

/// `tt0388629` from `One Piece [imdbid-tt0388629]`.
fn folder_imdb_id(name: &str) -> Option<String> {
    let start = name.find("[imdbid-")? + "[imdbid-".len();
    let id = name[start..].split(']').next()?;
    (id != "none" && kura_domain::is_imdb_id(id)).then(|| id.to_owned())
}

/// Collects `SxxEyy` and the language from video file names in `dir` and the folders below
/// it (seasons).
fn episodes_in(dir: &Path, depth: u8, found: &mut Vec<Found>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            if depth < 2 {
                episodes_in(&path, depth + 1, found);
            }
            continue;
        }
        let video = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| VIDEO.contains(&ext.to_ascii_lowercase().as_str()));
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(episode) = video.then(|| episode_code(&name)).flatten() {
            found.push(Found {
                episode,
                language: language_in(&name),
                path,
            });
        }
    }
}

/// Season and episode from a name containing `S01E001` or `s1e12`.
fn episode_code(name: &str) -> Option<(u32, u32)> {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for start in 0..bytes.len() {
        if bytes[start] != b's' {
            continue;
        }
        let digits = |from: usize| {
            let len = bytes[from..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            (len > 0).then(|| (lower[from..from + len].parse::<u32>().ok(), from + len))
        };
        let Some((Some(season), after)) = digits(start + 1) else {
            continue;
        };
        if bytes.get(after) != Some(&b'e') {
            continue;
        }
        if let Some((Some(episode), _)) = digits(after + 1) {
            return Some((season, episode));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_compare_by_letters_and_digits() {
        assert_eq!(title_key("One Piece [imdbid-tt0388629]"), "onepiece");
        assert_eq!(title_key("one-piece!"), "onepiece");
        assert_eq!(title_key("Re Zero  Part 2"), title_key("Re:Zero / Part 2"));
        assert_eq!(title_key("ReZero (2016)"), title_key("Re:Zero"));
        assert_eq!(title_key("[2016]"), "2016");
        assert_eq!(
            folder_imdb_id("One Piece [imdbid-tt0388629]").as_deref(),
            Some("tt0388629")
        );
        assert_eq!(folder_imdb_id("One Piece [imdbid-none]"), None);
        assert_eq!(folder_imdb_id("One Piece"), None);
    }

    #[test]
    fn episode_codes_are_read_from_file_names() {
        assert_eq!(
            episode_code("One Piece S01E001 German Dub.mkv"),
            Some((1, 1))
        );
        assert_eq!(episode_code("One Piece - S10E120.mkv"), Some((10, 120)));
        assert_eq!(episode_code("show.s2e7.720p.mkv"), Some((2, 7)));
        assert_eq!(episode_code("Sons of Anarchy S03E04.mkv"), Some((3, 4)));
        assert_eq!(episode_code("Season 01"), None);
        assert_eq!(episode_code("poster.jpg"), None);
    }
}
