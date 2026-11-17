//! The repositories opened recently, newest first, kept across launches like
//! VS Code's "Recent" list on its welcome page.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::json_file;

/// How many folders the list keeps.
const LIMIT: usize = 50;
const FILE_NAME: &str = "recent.json";

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    folders: Vec<PathBuf>,
}

pub struct RecentFolders {
    /// The file the list is kept in; `None` when there is nowhere to keep it.
    file: Option<PathBuf>,
    folders: Vec<PathBuf>,
}

impl RecentFolders {
    /// The list in `~/Library/Application Support/Whit`.
    pub fn load() -> Self {
        Self::load_from(json_file::data_file(FILE_NAME))
    }

    /// The list kept in `file`. A missing or unreadable file is an empty list.
    pub fn load_from(file: Option<PathBuf>) -> Self {
        let folders = file
            .as_deref()
            .and_then(json_file::read::<Stored>)
            .map(|stored| stored.folders)
            .unwrap_or_default();
        Self { file, folders }
    }

    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    /// Puts `folder` first, dropping an older entry for it and the oldest
    /// entries past the limit.
    pub fn record(&mut self, folder: &Path) {
        self.folders.retain(|existing| existing != folder);
        self.folders.insert(0, folder.to_path_buf());
        self.folders.truncate(LIMIT);
        self.save();
    }

    pub fn remove(&mut self, folder: &Path) {
        self.folders.retain(|existing| existing != folder);
        self.save();
    }

    fn save(&self) {
        if let Some(file) = &self.file {
            let stored = Stored {
                version: 1,
                folders: self.folders.clone(),
            };
            json_file::write(file, &stored);
        }
    }
}

/// How the welcome page names a folder: its own name, and its parent with the
/// home directory shortened to `~`.
pub fn display_parts(folder: &Path, home: Option<&Path>) -> (String, String) {
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string());
    let parent = folder.parent().unwrap_or(folder);
    let parent = match home.and_then(|home| parent.strip_prefix(home).ok()) {
        Some(relative) if relative.as_os_str().is_empty() => "~".to_owned(),
        Some(relative) => format!("~/{}", relative.display()),
        None => parent.display().to_string(),
    };
    (name, parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn list(directory: &Path) -> RecentFolders {
        RecentFolders::load_from(Some(directory.join(FILE_NAME)))
    }

    #[test]
    fn recording_moves_a_folder_first_and_persists() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut recent = list(directory.path());
        for name in ["/a", "/b", "/c"] {
            recent.record(Path::new(name));
        }
        recent.record(Path::new("/a"));
        assert_eq!(
            list(directory.path()).folders(),
            [Path::new("/a"), Path::new("/c"), Path::new("/b")]
        );

        recent.remove(Path::new("/c"));
        assert_eq!(
            list(directory.path()).folders(),
            [Path::new("/a"), Path::new("/b")]
        );
    }

    #[test]
    fn the_list_keeps_the_newest_fifty() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut recent = list(directory.path());
        for ix in 0..60 {
            recent.record(&PathBuf::from(format!("/folder{ix}")));
        }
        let folders = list(directory.path()).folders().to_vec();
        assert_eq!(folders.len(), LIMIT);
        assert_eq!(folders[0], Path::new("/folder59"));
        assert_eq!(folders[LIMIT - 1], Path::new("/folder10"));
    }

    #[test]
    fn a_missing_or_damaged_file_is_an_empty_list() {
        let directory = tempfile::tempdir().expect("temporary directory");
        assert!(list(directory.path()).folders().is_empty());
        fs::write(directory.path().join(FILE_NAME), "{ not json").expect("write");
        let mut recent = list(directory.path());
        assert!(recent.folders().is_empty());
        // Recording replaces the damaged file with a readable one.
        recent.record(Path::new("/a"));
        assert_eq!(list(directory.path()).folders(), [Path::new("/a")]);
    }

    #[test]
    fn folders_are_named_with_a_shortened_parent() {
        let home = Path::new("/Users/me");
        assert_eq!(
            display_parts(Path::new("/Users/me/Desktop/work/zed-lite"), Some(home)),
            ("zed-lite".into(), "~/Desktop/work".into())
        );
        assert_eq!(
            display_parts(Path::new("/Users/me/project"), Some(home)),
            ("project".into(), "~".into())
        );
        assert_eq!(
            display_parts(Path::new("/opt/code/app"), Some(home)),
            ("app".into(), "/opt/code".into())
        );
    }
}
