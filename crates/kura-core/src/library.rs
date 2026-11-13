//! A read-only view of the folders downloads go to.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::{Core, CoreError, Result};

/// A folder the Library page can open.
#[derive(Debug, PartialEq, Serialize)]
pub struct LibraryRoot {
    pub id: usize,
    pub name: String,
    pub path: String,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Entry {
    pub name: String,
    pub folder: bool,
    /// Bytes; 0 for folders.
    pub size: u64,
}

impl Core {
    /// The libraries from the settings, main library first.
    pub fn library_roots(&self) -> Result<Vec<LibraryRoot>> {
        let libraries = self.store.settings()?.libraries.into_iter().enumerate();
        let roots = libraries.map(|(id, library)| LibraryRoot {
            id,
            name: library.name,
            path: library.path.to_string_lossy().into_owned(),
        });
        Ok(roots.collect())
    }

    /// What is inside `relative`, a folder below one of the roots: folders first, then files,
    /// each by name. A folder that does not exist yet is empty.
    pub fn browse(&self, root: usize, relative: &str) -> Result<Vec<Entry>> {
        let outside = CoreError::Invalid("this is not a folder inside the library");
        let unreadable = |_| CoreError::Invalid("this folder cannot be read");
        let roots = self.library_roots()?;
        let root = roots
            .get(root)
            .ok_or(CoreError::Invalid("no such library"))?;
        let root = PathBuf::from(&root.path);

        // Only plain names: no `..`, no absolute path.
        let relative = Path::new(relative);
        let plain = |part| matches!(part, Component::Normal(_));
        if !relative.components().all(plain) {
            return Err(outside);
        }
        let folder = root.join(relative);
        if !folder.exists() {
            return Ok(Vec::new());
        }
        // A link inside the library could still point somewhere else.
        let real_root = root.canonicalize().map_err(unreadable)?;
        let real_folder = folder.canonicalize().map_err(unreadable)?;
        if !real_folder.starts_with(real_root) {
            return Err(outside);
        }

        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&folder).map_err(unreadable)? {
            let entry = entry.map_err(unreadable)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Hidden files and downloads still in progress are not part of the library.
            if name.starts_with('.') || name.ends_with(".part") {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(entry.path()) else {
                continue;
            };
            entries.push(Entry {
                name,
                folder: metadata.is_dir(),
                size: if metadata.is_dir() { 0 } else { metadata.len() },
            });
        }
        entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.cmp(&b.name)));
        Ok(entries)
    }
}
