//! What is already downloaded: finished downloads in the database, and series folders found
//! in the libraries, matched by IMDb ID or by name. Folders count even when Kura did not
//! create them, so a library filled by hand or by another tool is recognised too. File names
//! also tell the language an episode is in.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};

use kura_domain::{JobState, Language, SeriesSummary, SiteId};

use crate::{Core, Result, naming::language_in};

/// Library folders are read at most this often; pages ask for every list they show.
const SCAN_EVERY: Duration = Duration::from_secs(30);

const VIDEO: [&str; 6] = ["mkv", "mp4", "m4v", "avi", "webm", "ts"];

/// A series folder in a library.
#[derive(Debug, Clone)]
pub(crate) struct TitleFolder {
    path: PathBuf,
    key: String,
    imdb_id: Option<String>,
}

pub(crate) type Scan = (Instant, Arc<Vec<TitleFolder>>);

/// A downloaded `(season, episode)` and its language, when the record or file name tells it.
pub(crate) type Found = ((u32, u32), Option<Language>);

impl Core {
    /// Sets `downloaded` on every series of which at least one episode is downloaded. Lists
    /// carry no IMDb ID, so series found by ID on their own page count from then on.
    pub(crate) fn mark_downloaded(&self, list: &mut [SeriesSummary]) -> Result<()> {
        let jobs = self.store.jobs()?;
        let done: HashSet<(SiteId, &str)> = jobs
            .iter()
            .filter(|job| job.state == JobState::Done)
            .map(|job| (job.episode.site, job.episode.slug.as_str()))
            .collect();
        let folders = self.title_folders()?;
        let keys: HashSet<&str> = folders.iter().map(|folder| folder.key.as_str()).collect();
        let found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        for series in list {
            series.downloaded = done.contains(&(series.site, series.slug.as_str()))
                || keys.contains(title_key(&series.title).as_str())
                || found.contains(&(series.site, series.slug.clone()));
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
        Ok(found.into_iter().map(|(pair, _)| pair).collect())
    }

    /// Like [`Self::downloaded_episodes`], with the language of each episode where known.
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
            // Downloads from before 1.2.2 recorded only the picked language; the name has it.
            let named = path
                .and_then(Path::file_name)
                .and_then(|name| language_in(&name.to_string_lossy()));
            found.push((
                (job.episode.season, job.episode.number),
                job.language.or(named),
            ));
            // The folder a download went to, even if it matches neither ID nor name.
            if let Some(folder) = path.and_then(series_folder) {
                folders.push(folder);
            }
        }

        let key = title_key(title);
        let imdb_id = imdb_id.filter(|id| *id != "none");
        for folder in self.title_folders()?.iter() {
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
        // One entry per episode, the one with a known language if any.
        found.sort_by_key(|(pair, language)| (*pair, language.is_none()));
        found.dedup_by_key(|(pair, _)| *pair);
        if !found.is_empty() {
            let mut known = self.found.lock().unwrap_or_else(PoisonError::into_inner);
            known.insert((site, slug.to_owned()));
        }
        Ok(found)
    }

    /// Series folders directly in every library, and in a `Series` folder inside it where
    /// Kura 1.0 and 1.1 put them.
    fn title_folders(&self) -> Result<Arc<Vec<TitleFolder>>> {
        let lock = || self.scan.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((scanned, folders)) = lock().clone()
            && scanned.elapsed() < SCAN_EVERY
        {
            return Ok(folders);
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
                    folders.push(TitleFolder {
                        key: title_key(&name),
                        imdb_id: folder_imdb_id(&name),
                        path: entry.path(),
                    });
                }
            }
        }
        let folders = Arc::new(folders);
        *lock() = Some((Instant::now(), folders.clone()));
        Ok(folders)
    }

    /// Makes the next lookup read the library folders again, e.g. after a download.
    pub(crate) fn forget_scan(&self) {
        *self.scan.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
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
        if let Some(pair) = video.then(|| episode_code(&name)).flatten() {
            found.push((pair, language_in(&name)));
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
