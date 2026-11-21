//! The theme song: an MP3 an admin uploads, kept in the data folder. Kura ships no song of
//! its own, so nothing copyrighted is in the repository or the image.

use std::{path::PathBuf, sync::PoisonError, time::UNIX_EPOCH};

use crate::{Core, CoreError, Result};

/// Larger than any theme song; keeps a mistaken upload of a film out.
pub const MOST_MUSIC_BYTES: usize = 20 * 1024 * 1024;

/// The song and a tag that changes whenever it is replaced.
pub struct Music {
    pub bytes: Vec<u8>,
    pub etag: String,
}

impl Core {
    /// Where the data folder is; without one, there is no theme song.
    pub fn set_data_dir(&self, dir: PathBuf) {
        *self.data_dir.lock().unwrap_or_else(PoisonError::into_inner) = Some(dir);
    }

    fn music_path(&self) -> Option<PathBuf> {
        let dir = self.data_dir.lock().unwrap_or_else(PoisonError::into_inner);
        dir.as_ref().map(|dir| dir.join("theme").join("music.mp3"))
    }

    /// Whether an admin uploaded a theme song.
    pub fn has_music(&self) -> bool {
        self.music_path().is_some_and(|path| path.is_file())
    }

    /// The theme song, or `None` when there is none.
    pub fn music(&self) -> Result<Option<Music>> {
        let Some(path) = self.music_path() else {
            return Ok(None);
        };
        let modified = match std::fs::metadata(&path) {
            Ok(meta) => meta.modified().ok(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(folder_error(&path, err)),
        };
        let bytes = std::fs::read(&path).map_err(|err| folder_error(&path, err))?;
        let stamp = modified
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_millis());
        let etag = format!("\"{}-{stamp}\"", bytes.len());
        Ok(Some(Music { bytes, etag }))
    }

    /// Replaces the theme song. Only MP3 files are taken.
    pub fn save_music(&self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MOST_MUSIC_BYTES {
            return Err(CoreError::Invalid("the song may have at most 20 MB"));
        }
        if !is_mp3(bytes) {
            return Err(CoreError::Invalid("the song has to be an MP3 file"));
        }
        let path = self
            .music_path()
            .ok_or(CoreError::Invalid("Kura has no data folder"))?;
        let dir = path.parent().expect("the song lies in the theme folder");
        std::fs::create_dir_all(dir).map_err(|err| folder_error(dir, err))?;
        // Written next to it and renamed, so a page never plays half a file.
        let partial = dir.join("music.mp3.part");
        std::fs::write(&partial, bytes).map_err(|err| folder_error(&partial, err))?;
        std::fs::rename(&partial, &path).map_err(|err| folder_error(&path, err))
    }

    pub fn remove_music(&self) -> Result<()> {
        let Some(path) = self.music_path() else {
            return Ok(());
        };
        match std::fs::remove_file(&path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(folder_error(&path, err)),
            _ => Ok(()),
        }
    }
}

/// An ID3 tag, or an MPEG audio frame header.
fn is_mp3(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
}

fn folder_error(path: &std::path::Path, err: std::io::Error) -> CoreError {
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
}
