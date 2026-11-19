//! Watches a repository for changes made anywhere: the working tree and git's
//! own folder, through FSEvents on macOS. Events are gathered for a short
//! while and handed on in batches, as Zed's worktree does with its 100 ms
//! `FS_WATCH_LATENCY` (`crates/worktree/src/worktree.rs`, `crates/fs/src/
//! fs_watcher.rs`); this is a rewrite on the `notify` crate.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use notify::{EventKind, RecursiveMode, Watcher as _};

/// How long a batch keeps gathering after its first event.
pub const LATENCY: Duration = Duration::from_millis(100);

/// One changed path, relative to where it lives.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// In the working tree, relative to the root, `/`-separated.
    Worktree(String),
    /// In git's folder, relative to it, such as `HEAD` or `refs/heads/main`.
    Git(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub changes: BTreeSet<Change>,
    /// The system lost track of some events: anything may have changed.
    pub rescan: bool,
}

/// Stops watching when dropped.
pub struct RepositoryWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// The folders a repository's changes show up in.
struct Places {
    root: PathBuf,
    /// Git's folders: the repository's own, and for a linked working tree
    /// also the shared one; inside or outside the root.
    git_dirs: Vec<PathBuf>,
}

impl RepositoryWatcher {
    /// Watches `root` and `git_dirs`, sending a batch for every burst of
    /// changes until the watcher or the receiver is dropped.
    pub fn new(
        root: &Path,
        git_dirs: &[PathBuf],
    ) -> notify::Result<(Self, async_channel::Receiver<Batch>)> {
        // Events name real paths: `/var` is `/private/var` on macOS.
        let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let places = Places {
            root: canonical(root),
            git_dirs: git_dirs.iter().map(|dir| canonical(dir)).collect(),
        };
        let (events_tx, events_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(events_tx)?;
        watcher.watch(&places.root, RecursiveMode::Recursive)?;
        for dir in &places.git_dirs {
            if !dir.starts_with(&places.root) {
                watcher.watch(dir, RecursiveMode::Recursive)?;
            }
        }
        let batches = spawn_gathering("repository-watcher", events_rx, move |batch, event| {
            places.add(batch, event)
        });
        Ok((Self { _watcher: watcher }, batches))
    }
}

/// What a batch has gathered, to know whether it is worth sending.
trait Gathered: Default + Send + 'static {
    fn is_empty(&self) -> bool;
}

impl Gathered for Batch {
    fn is_empty(&self) -> bool {
        !self.rescan && self.changes.is_empty()
    }
}

/// Gathers the stream of events into batches on a thread of its own: the
/// first event opens a batch, which takes in everything arriving in the
/// next `LATENCY`. Ends when the watcher (the sender of events) or the
/// receiver of batches goes away.
fn spawn_gathering<B: Gathered>(
    name: &str,
    events: mpsc::Receiver<notify::Result<notify::Event>>,
    add: impl Fn(&mut B, notify::Result<notify::Event>) + Send + 'static,
) -> async_channel::Receiver<B> {
    let (batches, receiver) = async_channel::unbounded();
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Ok(first) = events.recv() {
                let mut batch = B::default();
                add(&mut batch, first);
                let deadline = Instant::now() + LATENCY;
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    match events.recv_timeout(left) {
                        Ok(event) => add(&mut batch, event),
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                if !batch.is_empty() && batches.send_blocking(batch).is_err() {
                    return;
                }
            }
        })
        .expect("start a watcher thread");
    receiver
}

/// The files that changed right inside the watched folders.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileBatch {
    /// Real paths, as the system names them.
    pub paths: BTreeSet<PathBuf>,
    /// The system lost track of some events: any file may have changed.
    pub rescan: bool,
}

impl Gathered for FileBatch {
    fn is_empty(&self) -> bool {
        !self.rescan && self.paths.is_empty()
    }
}

/// Watches folders for changes to the files right in them, not below: the
/// folder of the settings file, or of files open from outside the
/// repository. A folder is watched rather than the file, because editors
/// save by writing another file and renaming it over the old one, which a
/// watch on the file itself would lose track of; Zed watches its config
/// folder for the same reason (`crates/settings/src/settings_file.rs`,
/// `watch_config_dir`). This is a rewrite on the `notify` crate.
pub struct DirWatcher {
    watcher: notify::RecommendedWatcher,
}

impl DirWatcher {
    /// Watches nothing yet; `watch` adds folders.
    pub fn new() -> notify::Result<(Self, async_channel::Receiver<FileBatch>)> {
        let (events_tx, events_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let watcher = notify::recommended_watcher(events_tx)?;
        let batches = spawn_gathering("folder-watcher", events_rx, |batch: &mut FileBatch, event| {
            match event {
                Ok(event) => {
                    batch.rescan |= event.need_rescan();
                    if !matches!(event.kind, EventKind::Access(_)) {
                        batch.paths.extend(event.paths);
                    }
                }
                Err(error) => {
                    log::warn!("folder watcher error: {error}");
                    batch.rescan = true;
                }
            }
        });
        Ok((Self { watcher }, batches))
    }

    /// Watches the files right in `folder`, by its real path.
    pub fn watch(&mut self, folder: &Path) -> notify::Result<()> {
        self.watcher.watch(folder, RecursiveMode::NonRecursive)
    }

    pub fn unwatch(&mut self, folder: &Path) -> notify::Result<()> {
        self.watcher.unwatch(folder)
    }
}

impl Places {
    fn add(&self, batch: &mut Batch, event: notify::Result<notify::Event>) {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                log::warn!("file watcher error: {error}");
                batch.rescan = true;
                return;
            }
        };
        if event.need_rescan() {
            batch.rescan = true;
        }
        // Reading a file changes nothing.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        batch
            .changes
            .extend(event.paths.iter().filter_map(|path| self.change(path)));
    }

    fn change(&self, path: &Path) -> Option<Change> {
        let relative = |base: &Path| -> Option<String> {
            let rest = path.strip_prefix(base).ok()?;
            Some(
                rest.components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            )
        };
        if let Some(rest) = self.git_dirs.iter().find_map(|dir| relative(dir)) {
            return Some(Change::Git(rest));
        }
        relative(&self.root).map(Change::Worktree)
    }
}

/// What a batch asks of the window.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Read the changes, branch and sync state again.
    pub refresh: bool,
    /// Folders whose entries may have changed, relative to the root (`""`
    /// is the root); the explorer re-reads the expanded ones.
    pub folders: BTreeSet<String>,
    /// Paths that may have changed; open files among them are checked.
    pub files: BTreeSet<String>,
    /// The system lost events: re-read every folder, check every file.
    pub everything: bool,
}

/// Sorts a batch by what each change affects. Changes only under ignored
/// paths (build output) leave the change list alone, so a build does not
/// keep running `git status`; git's folder matters only where it keeps the
/// branch, the index and the refs, not its objects, logs or locks.
pub fn classify(batch: &Batch, is_ignored: impl Fn(&str) -> bool) -> Effects {
    let mut effects = Effects {
        refresh: batch.rescan,
        everything: batch.rescan,
        ..Effects::default()
    };
    for change in &batch.changes {
        match change {
            Change::Git(path) => effects.refresh |= tracks_git_state(path),
            Change::Worktree(path) if path.is_empty() => {}
            Change::Worktree(path) => {
                effects.refresh |= !is_ignored(path);
                let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
                effects.folders.insert(folder.to_owned());
                effects.files.insert(path.clone());
            }
        }
    }
    effects
}

/// Whether a path in git's folder holds state the window shows.
fn tracks_git_state(path: &str) -> bool {
    if path.ends_with(".lock") {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [
            "HEAD" | "index" | "packed-refs" | "MERGE_HEAD" | "CHERRY_PICK_HEAD" | "REVERT_HEAD"
            | "ORIG_HEAD" | "FETCH_HEAD",
        ] => true,
        ["refs" | "rebase-merge" | "rebase-apply", ..] => true,
        // A linked working tree keeps its own HEAD and index.
        ["worktrees", _, "HEAD" | "index"] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Waits up to `timeout` for a batch that has `wanted`.
    fn wait_for(receiver: &async_channel::Receiver<Batch>, wanted: &Change, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            while let Ok(batch) = receiver.try_recv() {
                if batch.changes.contains(wanted) {
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_write_arrives_as_a_batch() {
        let root = tempfile::tempdir().expect("temporary directory");
        let git_dir = root.path().join(".git");
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        let (watcher, batches) =
            RepositoryWatcher::new(root.path(), std::slice::from_ref(&git_dir)).expect("watch");
        // FSEvents starts reporting a moment after the stream opens.
        thread::sleep(Duration::from_millis(200));

        fs::write(root.path().join("a.rs"), "a").unwrap();
        assert!(wait_for(&batches, &Change::Worktree("a.rs".into()), Duration::from_millis(500)));

        fs::create_dir_all(root.path().join("src/deep")).unwrap();
        fs::write(root.path().join("src/deep/b.rs"), "b").unwrap();
        assert!(wait_for(
            &batches,
            &Change::Worktree("src/deep/b.rs".into()),
            Duration::from_millis(500)
        ));

        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert!(wait_for(&batches, &Change::Git("HEAD".into()), Duration::from_millis(500)));

        // Dropped, the watcher stops: no batch arrives any more.
        drop(watcher);
        thread::sleep(Duration::from_millis(200));
        while batches.try_recv().is_ok() {}
        fs::write(root.path().join("late.rs"), "late").unwrap();
        assert!(!wait_for(
            &batches,
            &Change::Worktree("late.rs".into()),
            Duration::from_millis(400)
        ));
    }

    #[test]
    fn a_burst_is_one_batch() {
        let root = tempfile::tempdir().expect("temporary directory");
        let (_watcher, batches) = RepositoryWatcher::new(root.path(), &[]).expect("watch");
        thread::sleep(Duration::from_millis(200));

        for ix in 0..20 {
            fs::write(root.path().join(format!("{ix}.o")), "").unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut seen = BTreeSet::new();
        let mut count = 0;
        while seen.len() < 20 && Instant::now() < deadline {
            if let Ok(batch) = batches.try_recv() {
                count += 1;
                seen.extend(batch.changes);
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(seen.len(), 20);
        assert!(count <= 2, "{count} batches for one burst");
    }

    /// Waits up to `timeout` for a batch naming `wanted`.
    fn wait_for_file(
        receiver: &async_channel::Receiver<FileBatch>,
        wanted: &Path,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            while let Ok(batch) = receiver.try_recv() {
                if batch.paths.contains(wanted) {
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_folder_watcher_sees_files_replaced_in_it() {
        let folder = tempfile::tempdir().expect("temporary directory");
        let real = folder.path().canonicalize().unwrap();
        let settings = real.join("settings.json");
        fs::write(&settings, "{}").unwrap();
        let (mut watcher, batches) = DirWatcher::new().expect("watch");
        watcher.watch(&real).expect("watch the folder");
        thread::sleep(Duration::from_millis(200));

        // Saved as editors do: another file renamed over it.
        fs::write(real.join("settings.json.tmp"), "{ \"a\": 1 }").unwrap();
        fs::rename(real.join("settings.json.tmp"), &settings).unwrap();
        assert!(wait_for_file(&batches, &settings, Duration::from_millis(500)));
        // And again: the watch outlives the file it saw first.
        fs::write(real.join("settings.json.tmp"), "{ \"a\": 2 }").unwrap();
        fs::rename(real.join("settings.json.tmp"), &settings).unwrap();
        assert!(wait_for_file(&batches, &settings, Duration::from_millis(500)));

        // Other files in the folder come too; the caller picks its own.
        let recent = real.join("recent.json");
        fs::write(&recent, "[]").unwrap();
        assert!(wait_for_file(&batches, &recent, Duration::from_millis(500)));

        watcher.unwatch(&real).expect("unwatch");
        thread::sleep(Duration::from_millis(200));
        while batches.try_recv().is_ok() {}
        fs::write(&settings, "{}").unwrap();
        assert!(!wait_for_file(&batches, &settings, Duration::from_millis(400)));
    }

    fn batch(changes: &[Change], rescan: bool) -> Batch {
        Batch {
            changes: changes.iter().cloned().collect(),
            rescan,
        }
    }

    fn ignored(path: &str) -> bool {
        path == "target" || path.starts_with("target/")
    }

    fn worktree(path: &str) -> Change {
        Change::Worktree(path.into())
    }

    fn git(path: &str) -> Change {
        Change::Git(path.into())
    }

    #[test]
    fn git_state_refreshes_and_its_noise_does_not() {
        for path in [
            "HEAD",
            "index",
            "packed-refs",
            "refs/heads/main",
            "refs/remotes/origin/main",
            "MERGE_HEAD",
            "rebase-merge/done",
            "worktrees/other/HEAD",
        ] {
            assert!(classify(&batch(&[git(path)], false), ignored).refresh, "{path}");
        }
        for path in [
            "index.lock",
            "refs/heads/main.lock",
            "objects/ab/cdef",
            "logs/HEAD",
            "COMMIT_EDITMSG",
            "worktrees/other/logs/HEAD",
        ] {
            let effects = classify(&batch(&[git(path)], false), ignored);
            assert_eq!(effects, Effects::default(), "{path}");
        }
    }

    #[test]
    fn worktree_changes_refresh_unless_ignored() {
        let effects = classify(&batch(&[worktree("src/a.rs")], false), ignored);
        assert!(effects.refresh);
        assert_eq!(effects.folders, BTreeSet::from(["src".to_owned()]));
        assert_eq!(effects.files, BTreeSet::from(["src/a.rs".to_owned()]));

        // A build writing a thousand files under an ignored folder.
        let build: Vec<Change> = (0..1000)
            .map(|ix| worktree(&format!("target/debug/deps/{ix}.o")))
            .collect();
        let effects = classify(&batch(&build, false), ignored);
        assert!(!effects.refresh);
        assert_eq!(effects.folders, BTreeSet::from(["target/debug/deps".to_owned()]));

        // A file in the root lists the root as its folder.
        let effects = classify(&batch(&[worktree("README.md")], false), ignored);
        assert_eq!(effects.folders, BTreeSet::from([String::new()]));
    }

    #[test]
    fn a_rescan_means_everything() {
        let effects = classify(&batch(&[], true), ignored);
        assert!(effects.refresh && effects.everything);
    }
}
