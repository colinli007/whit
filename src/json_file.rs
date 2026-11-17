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

/// Writes a temporary file and renames it over `file`, so a crash mid-write
/// leaves the previous contents intact.
pub fn write<T: Serialize>(file: &Path, value: &T) {
    let written = file
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| {
            let temporary = file.with_extension("json.tmp");
            let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
            fs::write(&temporary, json)?;
            fs::rename(&temporary, file)
        });
    if let Err(error) = written {
        log::warn!("cannot save {}: {error}", file.display());
    }
}
