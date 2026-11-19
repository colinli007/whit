//! The tabs of the editor area as VS Code keeps them: in display order, at
//! most one preview tab that the next preview open replaces, and the order of
//! use, which decides the tab that takes over when the active one closes.
//!
//! Zed's `Pane` (`crates/workspace/src/pane.rs`: `items`, `preview_item_id`,
//! `activation_history`) has the same parts; this is a rewrite for the kinds
//! of tab Whit has, keyed by what they show.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::SharedString;

use crate::git::ChangeGroup;

/// What a tab shows, which is also what makes it unique.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TabKey {
    /// A file on disk, editable, by its absolute path: in the repository
    /// or not, it is the same tab whatever folder is open.
    File(Arc<Path>),
    /// A changed file of the open repository in one group, read-only; the
    /// path is relative to the root.
    Diff {
        group: ChangeGroup,
        path: SharedString,
    },
    /// The settings page; one in the window.
    Settings,
}

/// How a file's path shows in its tab, its tooltip and the breadcrumbs:
/// relative to the repository root inside it, from the home folder as `~`
/// outside, as VS Code shows a file outside the workspace.
pub(crate) fn display_path(path: &Path, root: Option<&Path>, home: Option<&Path>) -> String {
    let slashed = |relative: &Path| {
        relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    };
    if let Some(relative) = root.and_then(|root| path.strip_prefix(root).ok()) {
        return slashed(relative);
    }
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(relative) => format!("~/{}", slashed(relative)),
        None => path.to_string_lossy().into_owned(),
    }
}

/// The home folder `display_path` shortens to `~`.
pub(crate) fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

#[derive(Default)]
pub(crate) struct EditorGroup {
    /// In display order.
    tabs: Vec<TabKey>,
    preview: Option<TabKey>,
    /// Least recently active first; the last one is the active tab.
    used: Vec<TabKey>,
}

/// The tab an open displaced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// A new tab, or one that was open already.
    Tab,
    /// The new tab took the place of this preview tab.
    Replaced(TabKey),
}

impl EditorGroup {
    pub(crate) fn tabs(&self) -> &[TabKey] {
        &self.tabs
    }

    pub(crate) fn active(&self) -> Option<&TabKey> {
        self.used.last()
    }

    pub(crate) fn is_preview(&self, key: &TabKey) -> bool {
        self.preview.as_ref() == Some(key)
    }

    /// Opens `key` and makes it active. A tab that is open already is reused;
    /// opened as a preview, a new tab replaces the current preview tab in
    /// place, otherwise it goes right of the active tab. Opening a preview
    /// tab for good pins it.
    pub(crate) fn open(&mut self, key: TabKey, preview: bool) -> Opened {
        if self.tabs.contains(&key) {
            if !preview {
                self.pin(&key);
            }
            self.activate(&key);
            return Opened::Tab;
        }
        let replaced = match self.preview.take() {
            Some(previous) if preview => {
                let ix = self.position(&previous).expect("the preview tab is open");
                self.tabs[ix] = key.clone();
                self.used.retain(|used| used != &previous);
                Some(previous)
            }
            previous => {
                self.preview = previous;
                let ix = self
                    .active()
                    .and_then(|active| self.position(active))
                    .map_or(self.tabs.len(), |ix| ix + 1);
                self.tabs.insert(ix, key.clone());
                None
            }
        };
        if preview {
            self.preview = Some(key.clone());
        }
        self.activate(&key);
        replaced.map_or(Opened::Tab, Opened::Replaced)
    }

    /// Keeps `key` open when the next preview opens.
    pub(crate) fn pin(&mut self, key: &TabKey) {
        if self.is_preview(key) {
            self.preview = None;
        }
    }

    pub(crate) fn activate(&mut self, key: &TabKey) {
        if self.tabs.contains(key) {
            self.used.retain(|used| used != key);
            self.used.push(key.clone());
        }
    }

    /// Closes `key`; the most recently used other tab becomes active, as
    /// with VS Code's `workbench.editor.focusRecentEditorAfterClose`.
    pub(crate) fn close(&mut self, key: &TabKey) {
        self.tabs.retain(|tab| tab != key);
        self.used.retain(|used| used != key);
        self.pin(key);
    }

    fn position(&self, key: &TabKey) -> Option<usize> {
        self.tabs.iter().position(|tab| tab == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::SharedString` comes without GPUI's `test` macro, but keep
    // the built-in explicit as the other test modules do.
    use core::prelude::v1::test;

    fn file(path: &str) -> TabKey {
        TabKey::File(Path::new("/repo").join(path).into())
    }

    fn diff(path: &str) -> TabKey {
        TabKey::Diff {
            group: ChangeGroup::Changes,
            path: path.to_owned().into(),
        }
    }

    fn names(group: &EditorGroup) -> Vec<String> {
        group
            .tabs()
            .iter()
            .map(|tab| match tab {
                TabKey::File(path) => display_path(path, Some(Path::new("/repo")), None),
                TabKey::Diff { path, .. } => format!("{path} (diff)"),
                TabKey::Settings => "settings".to_owned(),
            })
            .collect()
    }

    #[test]
    fn files_and_diffs_are_separate_tabs() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), false);
        group.open(diff("b.rs"), false);
        group.open(diff("a.rs"), false);
        assert_eq!(names(&group), ["a.rs", "b.rs (diff)", "a.rs (diff)"]);
        assert_eq!(group.active(), Some(&diff("a.rs")));
    }

    #[test]
    fn opening_an_open_tab_activates_it() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), false);
        group.open(file("b.rs"), false);
        assert_eq!(group.open(file("a.rs"), false), Opened::Tab);
        assert_eq!(names(&group), ["a.rs", "b.rs"]);
        assert_eq!(group.active(), Some(&file("a.rs")));
    }

    #[test]
    fn new_tabs_go_right_of_the_active_one() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), false);
        group.open(file("b.rs"), false);
        group.activate(&file("a.rs"));
        group.open(file("c.rs"), false);
        assert_eq!(names(&group), ["a.rs", "c.rs", "b.rs"]);
    }

    #[test]
    fn a_preview_replaces_the_preview_in_place() {
        let mut group = EditorGroup::default();
        group.open(file("x.rs"), false);
        group.open(file("a.rs"), true);
        group.activate(&file("x.rs"));
        assert_eq!(
            group.open(file("b.rs"), true),
            Opened::Replaced(file("a.rs"))
        );
        assert_eq!(names(&group), ["x.rs", "b.rs"]);
        assert!(group.is_preview(&file("b.rs")));
        assert_eq!(group.active(), Some(&file("b.rs")));
    }

    #[test]
    fn a_pinned_preview_stays() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), true);
        group.pin(&file("a.rs"));
        group.open(file("b.rs"), true);
        assert_eq!(names(&group), ["a.rs", "b.rs"]);
        assert!(!group.is_preview(&file("a.rs")));
        assert!(group.is_preview(&file("b.rs")));
    }

    #[test]
    fn opening_a_preview_for_good_pins_it() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), true);
        group.open(file("a.rs"), false);
        assert!(!group.is_preview(&file("a.rs")));
        // Opening it as a preview again does not make it one.
        group.open(file("a.rs"), true);
        assert!(!group.is_preview(&file("a.rs")));
    }

    #[test]
    fn closing_activates_the_most_recently_used_tab() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), false);
        group.open(file("b.rs"), false);
        group.open(file("c.rs"), false);
        group.activate(&file("a.rs"));
        group.close(&file("a.rs"));
        assert_eq!(group.active(), Some(&file("c.rs")));
        assert_eq!(names(&group), ["b.rs", "c.rs"]);
    }

    #[test]
    fn closing_the_last_tab_leaves_none_active() {
        let mut group = EditorGroup::default();
        group.open(file("a.rs"), true);
        group.close(&file("a.rs"));
        assert!(group.tabs().is_empty());
        assert_eq!(group.active(), None);
        // The closed preview is gone: the next preview is a new tab.
        group.open(file("b.rs"), true);
        assert_eq!(names(&group), ["b.rs"]);
    }

    #[test]
    fn paths_show_from_the_root_or_home() {
        let root = Path::new("/Users/me/code/whit");
        let home = Path::new("/Users/me");
        let show = |path: &str| display_path(Path::new(path), Some(root), Some(home));
        assert_eq!(show("/Users/me/code/whit/src/main.rs"), "src/main.rs");
        assert_eq!(
            show("/Users/me/Library/Application Support/Whit/settings.json"),
            "~/Library/Application Support/Whit/settings.json"
        );
        assert_eq!(show("/etc/hosts"), "/etc/hosts");
        // Without a repository, files inside it are outside files too.
        assert_eq!(
            display_path(Path::new("/Users/me/code/whit/a.rs"), None, Some(home)),
            "~/code/whit/a.rs"
        );
        // A sibling whose name starts like the root is not inside it.
        assert_eq!(show("/Users/me/code/whit2/a.rs"), "~/code/whit2/a.rs");
    }
}
