//! Songs for the background music: MP3s admins upload, kept in the data folder. Kura ships
//! no song of its own, so nothing copyrighted is in the repository or the image.

use std::{
    path::{Path, PathBuf},
    sync::PoisonError,
    time::UNIX_EPOCH,
};

use kura_domain::Song;

use crate::{Core, CoreError, Result};

/// Larger than any theme song; keeps a mistaken upload of a film out.
pub const MOST_MUSIC_BYTES: usize = 20 * 1024 * 1024;

/// More songs than anyone picks from.
const MOST_SONGS: usize = 50;

/// The name the one song of Kura 1.3.0 gets when it joins the list.
const OLD_SONG: &str = "Wii Shop Channel Main Theme";

/// A song and a tag that changes whenever it is replaced.
pub struct Music {
    pub bytes: Vec<u8>,
    pub etag: String,
}

impl Core {
    /// Where the data folder is; without one, there are no songs. Moves the single song of
    /// Kura 1.3.0 into the list.
    pub fn set_data_dir(&self, dir: PathBuf) {
        *self.data_dir.lock().unwrap_or_else(PoisonError::into_inner) = Some(dir.clone());
        let old = dir.join("theme").join("music.mp3");
        if old.is_file() {
            let moved = std::fs::read(&old)
                .map_err(|err| folder_error(&old, err))
                .and_then(|bytes| self.add_song(OLD_SONG, &bytes))
                .and_then(|_| std::fs::remove_file(&old).map_err(|err| folder_error(&old, err)));
            if let Err(err) = moved {
                tracing::warn!("could not move the theme song into the song list: {err}");
            }
        }
    }

    fn songs_dir(&self) -> Option<PathBuf> {
        let dir = self.data_dir.lock().unwrap_or_else(PoisonError::into_inner);
        dir.as_ref().map(|dir| dir.join("theme").join("songs"))
    }

    /// The uploaded songs, in the order they were added.
    pub fn songs(&self) -> Result<Vec<Song>> {
        let Some(dir) = self.songs_dir() else {
            return Ok(Vec::new());
        };
        let list = dir.join("songs.json");
        match std::fs::read_to_string(&list) {
            Ok(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(folder_error(&list, err)),
        }
    }

    /// Adds a song; only MP3 files are taken. Returns it with its new id.
    pub fn add_song(&self, name: &str, bytes: &[u8]) -> Result<Song> {
        if bytes.len() > MOST_MUSIC_BYTES {
            return Err(CoreError::Invalid("a song may have at most 20 MB"));
        }
        if !is_mp3(bytes) {
            return Err(CoreError::Invalid("the song has to be an MP3 file"));
        }
        let name = song_name(name);
        let dir = self
            .songs_dir()
            .ok_or(CoreError::Invalid("Kura has no data folder"))?;
        // One change to the list at a time.
        let _writing = self
            .songs_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut songs = self.songs()?;
        if songs.len() >= MOST_SONGS {
            return Err(CoreError::Invalid(
                "there are enough songs; remove one first",
            ));
        }
        std::fs::create_dir_all(&dir).map_err(|err| folder_error(&dir, err))?;
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).map_err(|err| CoreError::Hashing(err.to_string()))?;
        let id: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        let file = dir.join(format!("{id}.mp3"));
        std::fs::write(&file, bytes).map_err(|err| folder_error(&file, err))?;
        let song = Song { id, name };
        songs.push(song.clone());
        write_list(&dir, &songs)?;
        Ok(song)
    }

    /// Returns whether a song was removed.
    pub fn remove_song(&self, id: &str) -> Result<bool> {
        let Some(dir) = self.songs_dir() else {
            return Ok(false);
        };
        let _writing = self
            .songs_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut songs = self.songs()?;
        let before = songs.len();
        songs.retain(|song| song.id != id);
        if songs.len() == before {
            return Ok(false);
        }
        write_list(&dir, &songs)?;
        let file = dir.join(format!("{id}.mp3"));
        match std::fs::remove_file(&file) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(folder_error(&file, err)),
            _ => Ok(true),
        }
    }

    /// One song's file, or `None` for an unknown id.
    pub fn song(&self, id: &str) -> Result<Option<Music>> {
        // Only ids from the list name a file, so no path can be smuggled in.
        if !self.songs()?.iter().any(|song| song.id == id) {
            return Ok(None);
        }
        let Some(dir) = self.songs_dir() else {
            return Ok(None);
        };
        let path = dir.join(format!("{id}.mp3"));
        let modified = std::fs::metadata(&path)
            .map_err(|err| folder_error(&path, err))?
            .modified()
            .ok();
        let bytes = std::fs::read(&path).map_err(|err| folder_error(&path, err))?;
        let stamp = modified
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_millis());
        let etag = format!("\"{id}-{}-{stamp}\"", bytes.len());
        Ok(Some(Music { bytes, etag }))
    }
}

/// Written next to it and renamed, so a reader never sees half a list.
fn write_list(dir: &Path, songs: &[Song]) -> Result<()> {
    let list = dir.join("songs.json");
    let partial = dir.join("songs.json.part");
    let json = serde_json::to_string_pretty(songs).expect("songs always serialise");
    std::fs::write(&partial, json).map_err(|err| folder_error(&partial, err))?;
    std::fs::rename(&partial, &list).map_err(|err| folder_error(&list, err))
}

/// The file name without its extension, as the list shows it.
fn song_name(name: &str) -> String {
    let name = name.trim();
    let name = name
        .strip_suffix(".mp3")
        .or_else(|| name.strip_suffix(".MP3"))
        .unwrap_or(name)
        .replace(['_'], " ");
    let name: String = name.trim().chars().take(100).collect();
    if name.is_empty() {
        "Untitled song".to_owned()
    } else {
        name
    }
}

/// An ID3 tag, or an MPEG audio frame header.
fn is_mp3(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
}

fn folder_error(path: &Path, err: std::io::Error) -> CoreError {
    CoreError::Folder(path.to_string_lossy().into_owned(), err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mp3_files_are_taken() {
        assert!(is_mp3(b"ID3\x04\x00rest"));
        assert!(is_mp3(&[0xFF, 0xFB, 0x90, 0x00]));
        assert!(!is_mp3(b"<html>"));
        assert!(!is_mp3(b"\x89PNG"));
        assert!(!is_mp3(b""));
    }

    #[test]
    fn songs_are_named_after_their_file() {
        assert_eq!(song_name("Wii_Shop_Channel.mp3"), "Wii Shop Channel");
        assert_eq!(song_name("  Theme.MP3 "), "Theme");
        assert_eq!(song_name(""), "Untitled song");
        assert_eq!(song_name(&"x".repeat(300)).len(), 100);
    }
}
