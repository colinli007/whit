//! The answers VS Code remembers — 「始终」「从不」「不再显示」 — kept under
//! VS Code's own setting names.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::json_file;

const FILE_NAME: &str = "settings.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Values {
    /// With nothing staged, commit every change without asking.
    #[serde(rename = "git.enableSmartCommit")]
    pub enable_smart_commit: bool,
    /// With nothing staged, ask whether to commit every change.
    #[serde(rename = "git.suggestSmartCommit")]
    pub suggest_smart_commit: bool,
    /// Ask before pulling and pushing.
    #[serde(rename = "git.confirmSync")]
    pub confirm_sync: bool,
}

/// VS Code's defaults.
impl Default for Values {
    fn default() -> Self {
        Self {
            enable_smart_commit: false,
            suggest_smart_commit: true,
            confirm_sync: true,
        }
    }
}

pub struct Settings {
    /// The file the settings are kept in; `None` when there is nowhere to
    /// keep them.
    file: Option<PathBuf>,
    values: Values,
}

impl Settings {
    /// The settings in `~/Library/Application Support/Whit`.
    pub fn load() -> Self {
        Self::load_from(json_file::data_file(FILE_NAME))
    }

    /// The settings kept in `file`; a missing or unreadable file, or a
    /// missing key, keeps VS Code's default.
    pub fn load_from(file: Option<PathBuf>) -> Self {
        let values = file
            .as_deref()
            .and_then(json_file::read)
            .unwrap_or_default();
        Self { file, values }
    }

    pub fn values(&self) -> &Values {
        &self.values
    }

    /// Changes the settings and saves them.
    pub fn update(&mut self, change: impl FnOnce(&mut Values)) {
        change(&mut self.values);
        if let Some(file) = &self.file {
            json_file::write(file, &self.values);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    fn settings(directory: &Path) -> Settings {
        Settings::load_from(Some(directory.join(FILE_NAME)))
    }

    #[test]
    fn a_change_persists_under_vs_code_names() {
        let directory = tempfile::tempdir().expect("temporary directory");
        assert_eq!(*settings(directory.path()).values(), Values::default());

        settings(directory.path()).update(|values| values.confirm_sync = false);
        assert!(!settings(directory.path()).values().confirm_sync);
        let json = fs::read_to_string(directory.path().join(FILE_NAME)).expect("read");
        assert!(json.contains("\"git.confirmSync\": false"), "{json}");
    }

    #[test]
    fn a_missing_key_or_damaged_file_keeps_the_defaults() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join(FILE_NAME);
        fs::write(&file, r#"{ "git.enableSmartCommit": true }"#).expect("write");
        assert_eq!(
            *settings(directory.path()).values(),
            Values {
                enable_smart_commit: true,
                ..Values::default()
            }
        );

        fs::write(&file, "{ not json").expect("write");
        assert_eq!(*settings(directory.path()).values(), Values::default());
    }
}
