//! The JSON files the app keeps in `~/Library/Application Support/Whit`.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};

/// `name` in the app's data folder; `None` when the system has none.
pub fn data_file(name: &str) -> Option<PathBuf> {
    dirs::data_dir().map(|directory| directory.join(crate::APP_NAME).join(name))
}

/// What `file` holds. A missing or unreadable file is `None`: losing what
/// was kept must never stop the app from starting.
pub fn read<T: DeserializeOwned>(file: &Path) -> Option<T> {
    match fs::read(file) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .inspect_err(|error| log::warn!("ignoring {}: {error}", file.display()))
            .ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            log::warn!("cannot read {}: {error}", file.display());
            None
        }
    }
}

/// Writes `value` over `file`; a failure is only logged.
pub fn write<T: Serialize>(file: &Path, value: &T) {
    let written = serde_json::to_string_pretty(value)
        .map_err(io::Error::other)
        .and_then(|json| write_text(file, &json));
    if let Err(error) = written {
        log::warn!("cannot save {}: {error}", file.display());
    }
}

/// Writes a temporary file and renames it over `file`, so a crash mid-write
/// leaves the previous contents intact. A `file` that is a symbolic link
/// (say into a dotfiles folder) stays one: its target is written.
pub fn write_text(file: &Path, text: &str) -> io::Result<()> {
    let target = fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = target.with_extension("json.tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, &target)
}

/// The real path of `path`, links resolved, as the file watcher names it;
/// for a file not made yet, its folder's real path with its name.
pub fn real_path(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(folder), Some(name)) => fs::canonicalize(folder)
            .unwrap_or_else(|_| folder.to_owned())
            .join(name),
        _ => path.to_owned(),
    }
}
