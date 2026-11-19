//! The explorer's tree as VS Code keeps it: a folder is read only when it is
//! expanded, and the rows on screen are the expanded folders walked depth
//! first. Zed's project panel builds its visible entries the same way, from a
//! set of expanded folders, rebuilding the whole list on every change
//! (`crates/project_panel/src/project_panel.rs`, `update_visible_entries`);
//! this is a rewrite that reads folders on demand instead of scanning ahead.
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use crate::git::{ChangeGroup, Snapshot, StatusLetter};

/// VS Code's `files.exclude` defaults: never shown.
const EXCLUDED: &[&str] = &[".git", ".svn", ".hg", "CVS", ".DS_Store", "Thumbs.db"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// One line of the tree on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// Relative to the root, `/`-separated.
    pub path: String,
    pub name: String,
    /// 0 for the root's own entries.
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

#[derive(Default)]
pub(crate) struct Explorer {
    /// The entries of every folder read so far, by path (`""` is the root).
    children: HashMap<String, Vec<DirEntry>>,
    expanded: HashSet<String>,
    rows: Vec<Row>,
    pub selected: Option<String>,
}

impl Explorer {
    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn is_expanded(&self, path: &str) -> bool {
        self.expanded.contains(path)
    }

    /// Whether `path` was read; an expanded folder shows once it is.
    pub(crate) fn is_read(&self, path: &str) -> bool {
        self.children.contains_key(path)
    }

    /// The expanded folders and the root, whose entries are on screen.
    pub(crate) fn shown_folders(&self) -> impl Iterator<Item = &str> {
        std::iter::once("").chain(self.expanded.iter().map(String::as_str))
    }

    /// Takes in what `read_dir` found in `folder`.
    pub(crate) fn set_children(&mut self, folder: &str, entries: Vec<DirEntry>) {
        self.children.insert(folder.to_owned(), entries);
        self.rebuild();
    }

    pub(crate) fn expand(&mut self, folder: &str) {
        if self.expanded.insert(folder.to_owned()) {
            self.rebuild();
        }
    }

    pub(crate) fn collapse(&mut self, folder: &str) {
        if self.expanded.remove(folder) {
            self.rebuild();
        }
    }

    pub(crate) fn collapse_all(&mut self) {
        self.expanded.clear();
        self.rebuild();
    }

    /// Drops what was read of `folder`, which changed while out of sight;
    /// expanding it reads it again.
    pub(crate) fn forget(&mut self, folder: &str) {
        self.children.remove(folder);
    }

    /// The row of `path` and its index, when it is on screen.
    pub(crate) fn row(&self, path: &str) -> Option<(usize, &Row)> {
        self.rows.iter().enumerate().find(|(_, row)| row.path == path)
    }

    /// Walks the expanded folders depth first. Costs the number of entries
    /// in expanded folders, however large the repository.
    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        self.push_rows("", 0, &mut rows);
        self.rows = rows;
    }

    fn push_rows(&self, folder: &str, depth: usize, rows: &mut Vec<Row>) {
        let Some(entries) = self.children.get(folder) else {
            return;
        };
        for entry in entries {
            let path = join(folder, &entry.name);
            let expanded = entry.is_dir && self.expanded.contains(&path);
            rows.push(Row {
                path: path.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
            });
            if expanded {
                self.push_rows(&path, depth + 1, rows);
            }
        }
    }
}

pub(crate) fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The ancestors of `path`, outermost first: `a/b/c.rs` has `a` and `a/b`.
pub(crate) fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(|(ix, _)| &path[..ix])
}

/// The entries of `folder` under `root`, in VS Code's default order: folders
/// first, then files, each by name in natural order ignoring case (`file2`
/// before `file10`). An unreadable folder has none.
pub(crate) fn read_dir(root: &Path, folder: &str) -> Vec<DirEntry> {
    let Ok(read) = fs::read_dir(root.join(folder)) else {
        return Vec::new();
    };
    let mut entries: Vec<DirEntry> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if EXCLUDED.contains(&name.as_str()) {
                return None;
            }
            // Follows symbolic links, as VS Code shows a linked folder as one.
            let is_dir = fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir());
            Some(DirEntry { name, is_dir })
        })
        .collect();
    entries.sort_by(compare);
    entries
}

fn compare(a: &DirEntry, b: &DirEntry) -> Ordering {
    b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| natord::compare_ignore_case(&a.name, &b.name))
        .then_with(|| a.name.cmp(&b.name))
}

/// The git status the explorer shows for each path, folders included.
#[derive(Default)]
pub(crate) struct Decorations(HashMap<String, StatusLetter>);

impl Decorations {
    /// A file shows its unstaged status over its staged one; a folder shows
    /// the most urgent status in it: conflict, untracked, deleted, modified,
    /// then added.
    pub(crate) fn of(snapshot: &Snapshot) -> Self {
        let mut files: HashMap<&str, StatusLetter> = HashMap::new();
        for changed in &snapshot.files {
            let path = changed.file.path().as_ref();
            let unstaged_wins = changed.group != ChangeGroup::Staged;
            if unstaged_wins || !files.contains_key(path) {
                files.insert(path, changed.letter);
            }
        }
        let mut decorations: HashMap<String, StatusLetter> = HashMap::new();
        for (path, letter) in files {
            decorations.insert(path.to_owned(), letter);
            for folder in ancestors(path) {
                let shown = decorations.entry(folder.to_owned()).or_insert(letter);
                if urgency(letter) > urgency(*shown) {
                    *shown = letter;
                }
            }
        }
        Self(decorations)
    }

    pub(crate) fn get(&self, path: &str) -> Option<StatusLetter> {
        self.0.get(path).copied()
    }
}

fn urgency(letter: StatusLetter) -> u8 {
    match letter {
        StatusLetter::Conflicted => 4,
        StatusLetter::Untracked => 3,
        StatusLetter::Deleted => 2,
        StatusLetter::Modified => 1,
        StatusLetter::Added | StatusLetter::Renamed | StatusLetter::Copied => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixtures::{commit_all, git, new_repository, write};
    use crate::git::load_snapshot;
    use core::prelude::v1::test;

    fn names(entries: &[DirEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn folders_first_in_natural_order_without_excluded_names() {
        let root = tempfile::tempdir().expect("temporary directory");
        let path = root.path();
        for name in ["b.rs", "A.rs", "file10.txt", "file2.txt", ".DS_Store", ".gitignore"] {
            write(path, name, "");
        }
        for folder in ["z", ".git", "src"] {
            fs::create_dir(path.join(folder)).unwrap();
        }
        std::os::unix::fs::symlink("src", path.join("linked")).unwrap();

        let entries = read_dir(path, "");
        assert_eq!(
            names(&entries),
            ["linked", "src", "z", ".gitignore", "A.rs", "b.rs", "file2.txt", "file10.txt"]
        );
        assert!(entries[0].is_dir, "a link to a folder is a folder");
        assert!(read_dir(path, "missing").is_empty());
    }

    fn explorer() -> Explorer {
        let entry = |name: &str, is_dir| DirEntry {
            name: name.into(),
            is_dir,
        };
        let mut explorer = Explorer::default();
        explorer.set_children("", vec![entry("src", true), entry("Cargo.toml", false)]);
        explorer.set_children("src", vec![entry("git", true), entry("main.rs", false)]);
        explorer.set_children("src/git", vec![entry("mod.rs", false)]);
        explorer
    }

    fn shown(explorer: &Explorer) -> Vec<String> {
        explorer
            .rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
            .collect()
    }

    #[test]
    fn rows_follow_the_expanded_folders() {
        let mut explorer = explorer();
        assert_eq!(shown(&explorer), ["src", "Cargo.toml"]);

        explorer.expand("src");
        assert_eq!(shown(&explorer), ["src", "  git", "  main.rs", "Cargo.toml"]);
        explorer.expand("src/git");
        assert_eq!(
            shown(&explorer),
            ["src", "  git", "    mod.rs", "  main.rs", "Cargo.toml"]
        );

        // Collapsing hides the inside but remembers it.
        explorer.collapse("src");
        assert_eq!(shown(&explorer), ["src", "Cargo.toml"]);
        explorer.expand("src");
        assert_eq!(shown(&explorer).len(), 5);

        explorer.collapse_all();
        assert_eq!(shown(&explorer), ["src", "Cargo.toml"]);
        assert!(!explorer.is_expanded("src/git"));
    }

    #[test]
    fn ancestors_from_the_outside_in() {
        assert_eq!(ancestors("a/b/c.rs").collect::<Vec<_>>(), ["a", "a/b"]);
        assert!(ancestors("c.rs").next().is_none());
    }

    #[test]
    fn decorations_follow_the_unstaged_status_and_folders_the_most_urgent() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "src/staged.rs", "a\n");
        write(path, "src/both.rs", "a\n");
        write(path, "src/deep/mod.rs", "a\n");
        commit_all(path);
        // Staged only, then staged and changed again.
        write(path, "src/staged.rs", "b\n");
        write(path, "src/both.rs", "b\n");
        git(path, &["add", "src/staged.rs", "src/both.rs"]);
        write(path, "src/both.rs", "c\n");
        // Untracked deeper down.
        write(path, "src/deep/new.rs", "n\n");

        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        let decorations = Decorations::of(&snapshot);
        assert_eq!(decorations.get("src/staged.rs"), Some(StatusLetter::Modified));
        assert_eq!(decorations.get("src/both.rs"), Some(StatusLetter::Modified));
        assert_eq!(decorations.get("src/deep/new.rs"), Some(StatusLetter::Untracked));
        assert_eq!(decorations.get("src/deep"), Some(StatusLetter::Untracked));
        assert_eq!(decorations.get("src"), Some(StatusLetter::Untracked));
        assert_eq!(decorations.get("src/deep/mod.rs"), None);
    }
}
