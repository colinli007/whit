//! One refresh's worth of repository state: the branch line of the status bar
//! and every changed file, grouped the way VS Code's Source Control view groups
//! them. The file lists come from the same patches the diffs are drawn from.
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::component::diff::{DiffFile, DiffFileStatus};

use super::{GitError, git_command, run};

/// Git's well-known empty tree, used as the diff base before the first commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The viewer folds unchanged lines itself, so the patch carries whole files to
/// let every fold expand to the full source.
const DIFF_ARGS: &[&str] = &[
    "--no-color",
    // External diff and textconv programs come from repository configuration
    // and must not run just because a folder was opened.
    "--no-ext-diff",
    "--no-textconv",
    // Fixed prefixes override `diff.noprefix` and `diff.mnemonicPrefix`, which
    // would otherwise change the headers the patch parser expects.
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "-M",
    "--unified=1000000",
];

/// Unmerged paths have no two-sided diff against the index; they are read
/// separately, as the working file against HEAD. Without this filter the
/// staged diff reports them as `* Unmerged path` lines.
const WITHOUT_UNMERGED: &str = "--diff-filter=u";

/// How the working-tree diff shows an unmerged path whatever the filter: a
/// combined diff, which the parser does not read and the Merge group covers.
const COMBINED_DIFF: &str = "diff --cc ";

/// The groups of the Source Control view, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeGroup {
    /// Unmerged paths: the working file, conflict markers included, against HEAD.
    Merge,
    /// The index against HEAD.
    Staged,
    /// The working tree against the index, untracked files included.
    Changes,
}

/// The letter VS Code shows beside a changed file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusLetter {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Conflicted,
}

impl StatusLetter {
    pub fn as_str(self) -> &'static str {
        match self {
            StatusLetter::Modified => "M",
            StatusLetter::Added => "A",
            StatusLetter::Deleted => "D",
            StatusLetter::Renamed => "R",
            StatusLetter::Copied => "C",
            StatusLetter::Untracked => "U",
            StatusLetter::Conflicted => "!",
        }
    }
}

#[derive(Clone)]
pub struct ChangedFile {
    pub group: ChangeGroup,
    pub letter: StatusLetter,
    pub file: DiffFile,
    /// The part of the patch `file` was parsed from, which tells whether a
    /// reload changed this file.
    pub patch: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    /// A detached HEAD, shown by its abbreviated commit.
    Detached(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    /// Such as `origin/main`.
    pub name: String,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchStatus {
    pub head: Head,
    pub upstream: Option<Upstream>,
    pub has_commits: bool,
    /// Whether the index or the working tree differ from HEAD.
    pub dirty: bool,
}

#[derive(Clone)]
pub struct Snapshot {
    pub branch: BranchStatus,
    /// Merge changes, then staged changes, then changes; git's order within each.
    pub files: Vec<ChangedFile>,
    pub in_progress: Option<InProgress>,
    /// The message git prepared for the merge in progress.
    pub merge_message: Option<String>,
    /// The paths git ignores, as it lists them: an ignored folder is one
    /// entry ending in `/`, files inside it are not listed.
    pub ignored: Vec<String>,
    /// Everything the snapshot was built from; equal keys mean nothing changed.
    pub key: Arc<str>,
}

impl Snapshot {
    /// Whether git ignores `path` (relative to the root, `/`-separated):
    /// listed itself, or inside an ignored folder.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.ignored.iter().any(|ignored| match ignored.strip_suffix('/') {
            Some(folder) => {
                path == folder
                    || path
                        .strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/'))
            }
            None => path == ignored,
        })
    }
}

/// Reads the repository, or `None` when it is exactly as `shown` describes.
pub fn load_snapshot(root: &Path, shown: Option<&str>) -> Result<Option<Snapshot>, GitError> {
    let status_output = run(
        git_command(root).args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            // An ignored folder as one entry instead of every file in it.
            "--ignored=matching",
        ]),
        &[0],
    )?;
    let status = parse_status(&String::from_utf8_lossy(&status_output));
    let base = if status.branch.has_commits {
        "HEAD"
    } else {
        EMPTY_TREE
    };
    let staged = staged_patch(root, base)?;
    let changes = changes_patch(root, &status.untracked)?;
    let merge = merge_patch(root, &status.unmerged)?;
    let in_progress = in_progress(root)?;

    let key: Arc<str> = [
        &*String::from_utf8_lossy(&status_output),
        &staged,
        &changes,
        &merge,
        &format!("{in_progress:?}"),
    ]
    .join("\0")
    .into();
    if shown == Some(&*key) {
        return Ok(None);
    }
    let untracked: HashSet<&str> = status.untracked.iter().map(String::as_str).collect();
    let mut files = parse_group(&merge, ChangeGroup::Merge, |_| StatusLetter::Conflicted)?;
    files.extend(parse_group(&staged, ChangeGroup::Staged, letter_for)?);
    files.extend(parse_group(&changes, ChangeGroup::Changes, |file| {
        if untracked.contains(file.path().as_ref()) {
            StatusLetter::Untracked
        } else {
            letter_for(file)
        }
    })?);
    let merge_message = match in_progress {
        Some(InProgress::Merge) => merge_message(root)?,
        _ => None,
    };
    Ok(Some(Snapshot {
        branch: status.branch,
        files,
        in_progress,
        merge_message,
        ignored: status.ignored,
        key,
    }))
}

fn letter_for(file: &DiffFile) -> StatusLetter {
    match file.status() {
        DiffFileStatus::Added => StatusLetter::Added,
        DiffFileStatus::Deleted => StatusLetter::Deleted,
        DiffFileStatus::Renamed => StatusLetter::Renamed,
        DiffFileStatus::Copied => StatusLetter::Copied,
        DiffFileStatus::Conflicted => StatusLetter::Conflicted,
        _ => StatusLetter::Modified,
    }
}

fn parse_group(
    patch: &str,
    group: ChangeGroup,
    letter: impl Fn(&DiffFile) -> StatusLetter,
) -> Result<Vec<ChangedFile>, GitError> {
    let mut files = Vec::new();
    for section in file_sections(patch)
        .into_iter()
        .filter(|section| !section.starts_with(COMBINED_DIFF))
    {
        let parsed =
            DiffFile::parse(section).map_err(|error| GitError::Parse(error.to_string()))?;
        files.extend(parsed.into_iter().map(|file| ChangedFile {
            group,
            letter: letter(&file),
            file,
            patch: Arc::from(section),
        }));
    }
    Ok(files)
}

/// The index against HEAD (or the empty tree before the first commit).
fn staged_patch(root: &Path, base: &str) -> Result<String, GitError> {
    let patch = run(
        git_command(root)
            .args(["diff", "--cached", base, WITHOUT_UNMERGED])
            .args(DIFF_ARGS)
            .arg("--"),
        &[0],
    )?;
    Ok(String::from_utf8_lossy(&patch).into_owned())
}

/// The working tree against the index, untracked files shown as new files.
fn changes_patch(root: &Path, untracked: &[String]) -> Result<String, GitError> {
    let mut command = git_command(root);
    // Kept alive until the diff has run: it holds the index copy.
    let scratch;
    if !untracked.is_empty() {
        // Intent-to-add is git's own way of making `git diff` show untracked
        // files as new files: one diff covers them with git's binary detection,
        // modes and attributes, instead of one process per file. It is recorded
        // in a copy of the index so the user's staging area stays untouched.
        scratch = tempfile::tempdir().map_err(GitError::Io)?;
        let index = scratch.path().join("index");
        match fs::copy(index_path(root)?, &index) {
            Ok(_) => {}
            // Nothing has been staged yet; git creates the index on first add.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(GitError::Io(error)),
        }
        let pathspec = scratch.path().join("untracked");
        let paths: Vec<u8> = untracked
            .iter()
            .flat_map(|path| path.bytes().chain([0]))
            .collect();
        fs::write(&pathspec, paths).map_err(GitError::Io)?;
        run(
            git_command(root)
                .env("GIT_INDEX_FILE", &index)
                // Untracked names are paths, not glob patterns.
                .args(["--literal-pathspecs", "add", "--intent-to-add"])
                .arg("--pathspec-file-nul")
                .arg("--pathspec-from-file")
                .arg(&pathspec),
            &[0],
        )?;
        command.env("GIT_INDEX_FILE", index);
    }
    let patch = run(command.arg("diff").args(DIFF_ARGS).arg("--"), &[0])?;
    Ok(String::from_utf8_lossy(&patch).into_owned())
}

/// Each unmerged path's working file, conflict markers included, against HEAD.
fn merge_patch(root: &Path, unmerged: &[String]) -> Result<String, GitError> {
    if unmerged.is_empty() {
        return Ok(String::new());
    }
    let patch = run(
        git_command(root)
            .args(["--literal-pathspecs", "diff", "HEAD"])
            .args(DIFF_ARGS)
            .arg("--")
            .args(unmerged),
        &[0],
    )?;
    Ok(String::from_utf8_lossy(&patch).into_owned())
}

/// The index file of the repository, which linked worktrees keep outside `.git`.
fn index_path(root: &Path) -> Result<PathBuf, GitError> {
    let [index] = git_paths(root, ["index"])?;
    Ok(index)
}

/// Where git keeps the named files of this working tree's git directory, as
/// `rev-parse --git-path` resolves them (linked worktrees included).
fn git_paths<const N: usize>(root: &Path, names: [&str; N]) -> Result<[PathBuf; N], GitError> {
    let mut command = git_command(root);
    command.arg("rev-parse");
    for name in names {
        command.args(["--git-path", name]);
    }
    let output = run(&mut command, &[0])?;
    let output = String::from_utf8_lossy(&output);
    let paths: Vec<PathBuf> = output.lines().map(|line| root.join(line)).collect();
    paths
        .try_into()
        .map_err(|_| GitError::Parse(format!("unexpected rev-parse output: {output}")))
}

/// A merge or rebase that stopped and waits to be continued or aborted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    Rebase,
}

/// Reads the operation in progress from the files git itself keeps for it.
fn in_progress(root: &Path) -> Result<Option<InProgress>, GitError> {
    let [merge_head, rebase_merge, rebase_apply] =
        git_paths(root, ["MERGE_HEAD", "rebase-merge", "rebase-apply"])?;
    Ok(if rebase_merge.exists() || rebase_apply.exists() {
        Some(InProgress::Rebase)
    } else if merge_head.exists() {
        Some(InProgress::Merge)
    } else {
        None
    })
}

/// The message git prepared for the merge in progress, without its comment
/// lines, as VS Code puts it in the commit field.
fn merge_message(root: &Path) -> Result<Option<String>, GitError> {
    let [path] = git_paths(root, ["MERGE_MSG"])?;
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(None);
    };
    let message = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let message = message.trim();
    Ok((!message.is_empty()).then(|| message.to_owned()))
}

/// The commit HEAD points at, or `None` before the first commit.
pub fn head_commit(root: &Path) -> Result<Option<String>, GitError> {
    let output = run(
        git_command(root).args(["rev-parse", "--verify", "--quiet", "HEAD"]),
        &[0, 1],
    )?;
    let commit = String::from_utf8_lossy(&output).trim().to_owned();
    Ok((!commit.is_empty()).then_some(commit))
}

/// A configured remote, as `git remote -v` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub push_url: String,
}

/// The repository's remotes in git's order.
pub fn remotes(root: &Path) -> Result<Vec<Remote>, GitError> {
    let output = run(git_command(root).args(["remote", "-v"]), &[0])?;
    Ok(parse_remotes(&String::from_utf8_lossy(&output)))
}

/// Parses `<name>\t<url> (fetch|push)` lines, keeping each remote's push URL.
fn parse_remotes(output: &str) -> Vec<Remote> {
    output
        .lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            let url = rest.strip_suffix(" (push)")?;
            Some(Remote {
                name: name.to_owned(),
                push_url: url.to_owned(),
            })
        })
        .collect()
}

struct Status {
    branch: BranchStatus,
    untracked: Vec<String>,
    unmerged: Vec<String>,
    ignored: Vec<String>,
}

/// Parses `git status --porcelain=v2 --branch -z`.
fn parse_status(output: &str) -> Status {
    let mut oid = None;
    let mut head = None;
    let mut upstream_name = None;
    let mut ahead_behind = None;
    let mut untracked = Vec::new();
    let mut unmerged = Vec::new();
    let mut ignored = Vec::new();
    let mut dirty = false;
    let mut entries = output.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        if let Some(header) = entry.strip_prefix("# ") {
            let (name, value) = header.split_once(' ').unwrap_or((header, ""));
            match name {
                "branch.oid" => oid = Some(value.to_owned()),
                "branch.head" => head = Some(value.to_owned()),
                "branch.upstream" => upstream_name = Some(value.to_owned()),
                "branch.ab" => ahead_behind = parse_ahead_behind(value),
                _ => {}
            }
            continue;
        }
        // Fields are space-separated up to the path, which may hold spaces.
        match entry.as_bytes().first() {
            Some(b'1') => dirty = true,
            Some(b'2') => {
                dirty = true;
                // A rename's original path follows as its own entry.
                entries.next();
            }
            Some(b'u') => {
                dirty = true;
                if let Some(path) = entry.splitn(11, ' ').nth(10) {
                    unmerged.push(path.to_owned());
                }
            }
            Some(b'?') => {
                dirty = true;
                untracked.push(entry[2..].to_owned());
            }
            Some(b'!') => ignored.push(entry[2..].to_owned()),
            _ => {}
        }
    }
    let has_commits = oid.as_deref().is_some_and(|oid| oid != "(initial)");
    let head = match head.as_deref() {
        Some("(detached)") | None => Head::Detached(
            oid.as_deref()
                .map(|oid| oid.chars().take(8).collect())
                .unwrap_or_default(),
        ),
        Some(name) => Head::Branch(name.to_owned()),
    };
    // A gone upstream reports its name without counts; it cannot be synced.
    let upstream = upstream_name
        .zip(ahead_behind)
        .map(|(name, (ahead, behind))| Upstream {
            name,
            ahead,
            behind,
        });
    Status {
        branch: BranchStatus {
            head,
            upstream,
            has_commits,
            dirty,
        },
        untracked,
        unmerged,
        ignored,
    }
}

/// Parses the `+<ahead> -<behind>` of a `branch.ab` header.
fn parse_ahead_behind(value: &str) -> Option<(u32, u32)> {
    let (ahead, behind) = value.split_once(' ')?;
    Some((
        ahead.strip_prefix('+')?.parse().ok()?,
        behind.strip_prefix('-')?.parse().ok()?,
    ))
}

/// Splits a patch into one section per file. Each starts at its `diff --git`
/// line; hunk lines begin with ` `, `+`, `-` or `\`, so they never match.
fn file_sections(patch: &str) -> Vec<&str> {
    let starts = patch.match_indices("\ndiff --git ").map(|(ix, _)| ix + 1);
    let bounds: Vec<usize> = std::iter::once(0)
        .chain(starts)
        .chain(std::iter::once(patch.len()))
        .collect();
    bounds
        .windows(2)
        .map(|range| &patch[range[0]..range[1]])
        .filter(|section| !section.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{
        add_remote, clone_of, commit_all, git, git_output, new_repository, write,
    };
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn snapshot(root: &Path) -> Snapshot {
        load_snapshot(root, None)
            .expect("load snapshot")
            .expect("a snapshot")
    }

    /// Group, path, letter, additions and deletions of every file.
    fn summary(snapshot: &Snapshot) -> Vec<(ChangeGroup, String, &'static str, usize, usize)> {
        snapshot
            .files
            .iter()
            .map(|changed| {
                (
                    changed.group,
                    changed.file.path().to_string(),
                    changed.letter.as_str(),
                    changed.file.additions(),
                    changed.file.deletions(),
                )
            })
            .collect()
    }

    use ChangeGroup::{Changes, Merge, Staged};

    #[test]
    fn staged_and_unstaged_changes_are_separate_groups() {
        let repository = new_repository();
        let path = repository.path();
        let numbered: String = (1..=20).map(|line| format!("line {line}\n")).collect();
        write(path, "modified.txt", &numbered);
        write(path, "deleted.txt", "gone\n");
        write(path, "old_name.txt", &numbered);
        commit_all(path);

        let first = numbered.replace("line 1\n", "line one\n");
        write(path, "modified.txt", &first);
        git(path, &["add", "modified.txt"]);
        write(
            path,
            "modified.txt",
            first.replace("line 10\n", "line ten\n"),
        );
        std::fs::remove_file(path.join("deleted.txt")).expect("delete file");
        git(path, &["mv", "old_name.txt", "new_name.txt"]);

        let snapshot = snapshot(path);
        assert_eq!(
            summary(&snapshot),
            vec![
                (Staged, "modified.txt".into(), "M", 1, 1),
                (Staged, "new_name.txt".into(), "R", 0, 0),
                (Changes, "deleted.txt".into(), "D", 0, 1),
                (Changes, "modified.txt".into(), "M", 1, 1),
            ]
        );
        // Each group's diff holds only its own part of a partly staged file.
        assert!(snapshot.files[0].patch.contains("+line one"));
        assert!(!snapshot.files[0].patch.contains("+line ten"));
        assert!(snapshot.files[3].patch.contains("+line ten"));
        assert!(!snapshot.files[3].patch.contains("+line one"));
        assert_eq!(
            snapshot.files[1]
                .file
                .original_path()
                .map(|path| path.to_string()),
            Some("old_name.txt".to_string())
        );
    }

    #[test]
    fn untracked_files_are_changes_and_ignored_files_are_skipped() {
        let repository = new_repository();
        let path = repository.path();
        write(path, ".gitignore", "ignored.txt\n");
        commit_all(path);

        write(path, "new.txt", "first\nsecond\n");
        write(path, "nested/also new 新.txt", "only\n");
        write(path, "ignored.txt", "hidden\n");
        write(path, "image.bin", [0u8, 159, 146, 150, 0, 1, 2]);
        write(path, "staged_new.txt", "s\n");
        git(path, &["add", "staged_new.txt"]);

        let snapshot = snapshot(path);
        assert_eq!(
            summary(&snapshot),
            vec![
                (Staged, "staged_new.txt".into(), "A", 1, 0),
                (Changes, "image.bin".into(), "U", 0, 0),
                (Changes, "nested/also new 新.txt".into(), "U", 1, 0),
                (Changes, "new.txt".into(), "U", 2, 0),
            ]
        );
        assert!(snapshot.files[1].file.is_binary());
    }

    #[test]
    fn loading_leaves_the_staging_area_untouched() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "tracked.txt", "a\n");
        commit_all(path);
        write(path, "untracked.txt", "b\n");
        let status = || git_output(path, &["status", "--porcelain"]);

        let before = status();
        snapshot(path);
        assert_eq!(status(), before);
    }

    #[test]
    fn repository_without_commits() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "staged.txt", "a\n");
        git(path, &["add", "staged.txt"]);
        write(path, "untracked.txt", "b\nc\n");

        let snapshot = snapshot(path);
        assert_eq!(
            summary(&snapshot),
            vec![
                (Staged, "staged.txt".into(), "A", 1, 0),
                (Changes, "untracked.txt".into(), "U", 2, 0),
            ]
        );
        assert!(!snapshot.branch.has_commits);
        assert_eq!(snapshot.branch.head, Head::Branch("main".into()));
        assert!(snapshot.branch.dirty);
    }

    #[test]
    fn clean_worktree_and_unchanged_reloads() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "file.txt", "content\n");
        commit_all(path);

        let clean = snapshot(path);
        assert!(clean.files.is_empty());
        assert!(!clean.branch.dirty);
        assert!(
            load_snapshot(path, Some(&clean.key))
                .expect("reload")
                .is_none()
        );
        write(path, "file.txt", "changed\n");
        assert!(
            load_snapshot(path, Some(&clean.key))
                .expect("reload")
                .is_some()
        );
    }

    #[test]
    fn user_diff_configuration_does_not_change_the_snapshot() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "file.txt", "before\n");
        commit_all(path);
        write(path, "file.txt", "after\n");
        write(path, "untracked.txt", "new\n");

        let expected = summary(&snapshot(path));
        git(path, &["config", "diff.noprefix", "true"]);
        git(path, &["config", "diff.mnemonicPrefix", "true"]);
        git(path, &["config", "color.ui", "always"]);
        git(path, &["config", "status.showUntrackedFiles", "no"]);
        assert_eq!(summary(&snapshot(path)), expected);
    }

    #[test]
    fn branch_tracks_its_upstream() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "file.txt", "one\n");
        commit_all(path);
        assert_eq!(snapshot(path).branch.upstream, None);

        let remote = add_remote(path);
        // Two commits only the remote has, one only the local branch has.
        let other = clone_of(remote.path());
        for content in ["two\n", "three\n"] {
            write(other.path(), "other.txt", content);
            commit_all(other.path());
        }
        git(other.path(), &["push", "--quiet"]);
        git(path, &["fetch", "--quiet"]);
        write(path, "file.txt", "local\n");
        commit_all(path);

        let branch = snapshot(path).branch;
        assert_eq!(branch.head, Head::Branch("main".into()));
        assert_eq!(
            branch.upstream,
            Some(Upstream {
                name: "origin/main".into(),
                ahead: 1,
                behind: 2,
            })
        );
        assert!(!branch.dirty);
    }

    #[test]
    fn detached_head_shows_its_commit() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "file.txt", "one\n");
        commit_all(path);
        let commit = git_output(path, &["rev-parse", "HEAD"]);
        git(path, &["checkout", "--quiet", "--detach"]);

        assert_eq!(
            snapshot(path).branch.head,
            Head::Detached(commit[..8].to_string())
        );
    }

    #[test]
    fn conflicted_files_are_merge_changes() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "both.txt", "base\n");
        write(path, "clean.txt", "base\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "-b", "theirs"]);
        write(path, "both.txt", "theirs\n");
        write(path, "clean.txt", "theirs\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "main"]);
        write(path, "both.txt", "ours\n");
        commit_all(path);
        let merged = Command::new("git")
            .current_dir(path)
            .args(["merge", "--quiet", "theirs"])
            .output()
            .expect("git merge");
        assert!(!merged.status.success(), "the merge should conflict");

        let snapshot = snapshot(path);
        assert_eq!(
            summary(&snapshot)
                .into_iter()
                .map(|(group, path, letter, ..)| (group, path, letter))
                .collect::<Vec<_>>(),
            vec![
                (Merge, "both.txt".into(), "!"),
                (Staged, "clean.txt".into(), "M"),
            ]
        );
        assert!(snapshot.files[0].patch.contains("+<<<<<<< HEAD"));
    }

    /// A repository whose `main` and `theirs` both changed `both.txt`.
    fn diverged_repository() -> TempDir {
        let repository = new_repository();
        let path = repository.path();
        write(path, "both.txt", "base\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "-b", "theirs"]);
        write(path, "both.txt", "theirs\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "main"]);
        write(path, "both.txt", "ours\n");
        commit_all(path);
        repository
    }

    fn git_fails(path: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .current_dir(path)
            .args(arguments)
            .output()
            .expect("git should run");
        assert!(!output.status.success(), "git {arguments:?} should fail");
    }

    #[test]
    fn a_stopped_merge_or_rebase_is_in_progress() {
        let repository = diverged_repository();
        let path = repository.path();
        assert_eq!(snapshot(path).in_progress, None);
        assert_eq!(merge_message(path).unwrap(), None);

        git_fails(path, &["merge", "--quiet", "theirs"]);
        assert_eq!(snapshot(path).in_progress, Some(InProgress::Merge));
        let message = merge_message(path).unwrap().expect("merge message");
        assert!(message.starts_with("Merge branch 'theirs'"), "{message}");
        assert!(!message.contains('#'), "{message}");
        git(path, &["merge", "--abort"]);
        assert_eq!(snapshot(path).in_progress, None);

        git_fails(path, &["rebase", "theirs"]);
        assert_eq!(snapshot(path).in_progress, Some(InProgress::Rebase));
        git(path, &["rebase", "--abort"]);
        assert_eq!(snapshot(path).in_progress, None);
    }

    #[test]
    fn progress_is_read_from_a_linked_worktree_too() {
        let repository = diverged_repository();
        let path = repository.path();
        let worktree = tempfile::tempdir().expect("temporary directory");
        let linked = worktree.path().join("linked");
        git(
            path,
            &[
                "worktree",
                "add",
                "--quiet",
                linked.to_str().unwrap(),
                "-b",
                "other",
                "main",
            ],
        );
        git_fails(&linked, &["merge", "--quiet", "theirs"]);
        assert_eq!(snapshot(&linked).in_progress, Some(InProgress::Merge));
        // The main working tree is not merging.
        assert_eq!(snapshot(path).in_progress, None);
    }

    #[test]
    fn status_parsing_handles_renames_and_spaces() {
        let output = [
            "# branch.oid 0123456789abcdef",
            "# branch.head topic",
            "# branch.upstream origin/topic",
            "# branch.ab +3 -0",
            "2 R. N... 100644 100644 100644 aaa bbb R100 new name.txt",
            "old name.txt",
            "? dir/untracked file.txt",
            "u UU N... 100644 100644 100644 100644 a b c conflict file.txt",
            "",
        ]
        .join("\0");
        let status = parse_status(&output);
        assert_eq!(status.branch.head, Head::Branch("topic".into()));
        assert_eq!(
            status.branch.upstream,
            Some(Upstream {
                name: "origin/topic".into(),
                ahead: 3,
                behind: 0
            })
        );
        assert!(status.branch.has_commits && status.branch.dirty);
        // The rename's original path is not mistaken for another entry.
        assert_eq!(status.untracked, ["dir/untracked file.txt"]);
        assert_eq!(status.unmerged, ["conflict file.txt"]);
    }

    #[test]
    fn patch_splits_into_one_section_per_file() {
        let patch = "diff --git a/a b/a\n@@ -1 +1 @@\n-diff --git x\n+y\ndiff --git a/b b/b\n+z\n";
        assert_eq!(
            file_sections(patch),
            [
                "diff --git a/a b/a\n@@ -1 +1 @@\n-diff --git x\n+y\n",
                "diff --git a/b b/b\n+z\n"
            ]
        );
        assert!(file_sections("").is_empty());
    }

    #[test]
    fn ignored_paths_are_listed_once_per_folder() {
        let repository = new_repository();
        let path = repository.path();
        write(path, ".gitignore", "target/\n*.log\n");
        commit_all(path);
        write(path, "target/debug/deep/a.o", "");
        write(path, "target/b.o", "");
        write(path, "app.log", "");
        write(path, "src/x.log", "");
        write(path, "target2/kept.rs", "");

        let snapshot = snapshot(path);
        let mut ignored = snapshot.ignored.clone();
        ignored.sort();
        assert_eq!(ignored, ["app.log", "src/x.log", "target/"]);

        assert!(snapshot.is_ignored("target"));
        assert!(snapshot.is_ignored("target/debug/deep/a.o"));
        assert!(snapshot.is_ignored("src/x.log"));
        assert!(!snapshot.is_ignored("target2"));
        assert!(!snapshot.is_ignored("target2/kept.rs"));
        assert!(!snapshot.is_ignored("src"));
        assert!(!snapshot.is_ignored("src/x.log.rs"));
    }

    /// Thousands of ignored files cost one status line, not one each.
    #[test]
    fn an_ignored_folder_costs_one_entry() {
        let repository = new_repository();
        let path = repository.path();
        write(path, ".gitignore", "target/\n");
        commit_all(path);
        for ix in 0..5000 {
            write(path, &format!("target/{}/{ix}.o", ix % 50), "");
        }

        let started = std::time::Instant::now();
        let snapshot = snapshot(path);
        println!("load_snapshot with 5000 ignored files: {:?}", started.elapsed());
        assert_eq!(snapshot.ignored, ["target/"]);
    }
}
