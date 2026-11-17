//! Runs the local `git`. Reading lives in [`read`], changes to the repository
//! in [`write`]; every command is built here so they share the same
//! environment and error classification.
mod branches;
mod read;
mod refs;
mod write;

use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

pub use branches::{
    checkout, checkout_detached, checkout_tracking, create_branch, delete_branch,
    delete_remote_branch, discard_everything, merge, merge_abort, rebase, rebase_abort,
    rebase_continue, rename_branch, stash_all, stash_pop,
};
pub use read::{
    BranchStatus, ChangeGroup, ChangedFile, Head, InProgress, Remote, Snapshot, StatusLetter,
    head_commit, load_snapshot, remotes,
};
pub use refs::{RefKind, Reference, list_references, sanitize_branch_name};
pub use write::{commit, discard, fetch, publish, pull, push, stage, stage_all, unstage};

#[derive(Debug)]
pub enum GitError {
    GitNotFound,
    NotARepository,
    CommandFailed {
        command: String,
        stderr: String,
    },
    Io(io::Error),
    /// Git produced a patch the diff parser does not understand.
    Parse(String),
    /// Publishing needs a remote and the repository has none.
    NoRemote,
    /// A checkout would overwrite local changes.
    DirtyWorkTree,
    /// `branch -d` refused a branch whose commits are not merged anywhere.
    BranchNotFullyMerged,
    /// A merge, rebase or stash pop stopped with unmerged paths.
    Conflict,
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::GitNotFound => write!(formatter, "找不到 git 命令，请先安装 git"),
            GitError::NotARepository => write!(formatter, "该文件夹不是 git 仓库"),
            GitError::CommandFailed { command, stderr } => {
                write!(formatter, "执行 {command} 失败：{}", stderr.trim())
            }
            GitError::Io(error) => write!(formatter, "执行 git 时出错：{error}"),
            GitError::Parse(error) => write!(formatter, "解析 diff 失败：{error}"),
            GitError::NoRemote => write!(formatter, "仓库未配置任何要发布到的远程仓库。"),
            GitError::DirtyWorkTree => write!(formatter, "签出会覆盖本地更改。"),
            GitError::BranchNotFullyMerged => write!(formatter, "分支未被完全合并。"),
            GitError::Conflict => {
                write!(formatter, "存在合并冲突。请在提交之前解决这些冲突。")
            }
        }
    }
}

impl std::error::Error for GitError {}

impl GitError {
    /// Whether git stopped because it needed credentials it could not ask for.
    pub fn needs_authentication(&self) -> bool {
        const SIGNS: &[&str] = &[
            "terminal prompts disabled",
            "could not read Username",
            "could not read Password",
            "Authentication failed",
            "Permission denied (publickey",
            "Host key verification failed",
        ];
        matches!(self, GitError::CommandFailed { stderr, .. }
            if SIGNS.iter().any(|sign| stderr.contains(sign)))
    }
}

/// Returns the working tree root of the repository containing `folder`.
pub fn repository_root(folder: &Path) -> Result<PathBuf, GitError> {
    let output = run(
        git_command(folder).args(["rev-parse", "--show-toplevel"]),
        &[0],
    )?;
    let root = String::from_utf8_lossy(&output);
    Ok(PathBuf::from(root.trim_end_matches('\n')))
}

/// What every git command shares, whether it reads or writes.
fn base_command(directory: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(directory)
        .stdin(Stdio::null())
        // Error classification reads git's messages, so keep them untranslated.
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("--no-pager")
        .args(["-c", "core.quotepath=false"]);
    command
}

/// A command that only reads. Opening a folder must neither run programs from
/// its configuration nor contend with the user's own git commands.
fn git_command(directory: &Path) -> Command {
    let mut command = base_command(directory);
    command
        // Reading must not take index.lock away from a git command the user is
        // running in a terminal at the same time.
        .arg("--no-optional-locks")
        // fsmonitor hooks are commands from repository configuration.
        .args(["-c", "core.fsmonitor=false"]);
    command
}

fn run(command: &mut Command, success_codes: &[i32]) -> Result<Vec<u8>, GitError> {
    let output = output(command)?;
    finish(command, output, success_codes)
}

/// Turns a finished command into its standard output or a classified error.
fn finish(command: &Command, output: Output, success_codes: &[i32]) -> Result<Vec<u8>, GitError> {
    match output.status.code() {
        Some(code) if success_codes.contains(&code) => Ok(output.stdout),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not a git repository") {
                Err(GitError::NotARepository)
            } else if stderr.trim().is_empty() {
                // Some refusals, such as "nothing to commit", go to stdout.
                Err(command_failed(command, &output.stdout))
            } else {
                Err(command_failed(command, &output.stderr))
            }
        }
    }
}

fn output(command: &mut Command) -> Result<Output, GitError> {
    command.output().map_err(spawn_failed)
}

fn spawn_failed(error: io::Error) -> GitError {
    match error.kind() {
        io::ErrorKind::NotFound => GitError::GitNotFound,
        _ => GitError::Io(error),
    }
}

fn command_failed(command: &Command, stderr: &[u8]) -> GitError {
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    GitError::CommandFailed {
        command: format!("git {arguments}"),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
    }
}

/// Temporary repositories for tests of this module and of the view.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::{fs, path::Path, process::Command};
    use tempfile::TempDir;

    pub(crate) fn git(directory: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .current_dir(directory)
            .args(arguments)
            .status()
            .expect("git should run");
        assert!(status.success(), "git {arguments:?} failed");
    }

    pub(crate) fn new_repository() -> TempDir {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path();
        git(path, &["init", "--quiet", "--initial-branch=main"]);
        git(path, &["config", "user.name", "Test"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["config", "commit.gpgsign", "false"]);
        directory
    }

    pub(crate) fn commit_all(path: &Path) {
        git(path, &["add", "--all"]);
        git(path, &["commit", "--quiet", "--message", "commit"]);
    }

    /// Standard output of a git command that must succeed.
    pub(crate) fn git_output(directory: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(directory)
            .args(arguments)
            .output()
            .expect("git should run");
        assert!(output.status.success(), "git {arguments:?} failed");
        String::from_utf8(output.stdout).expect("utf-8 output")
    }

    /// A bare repository added as `origin`, with `main` pushed and tracked.
    pub(crate) fn add_remote(path: &Path) -> TempDir {
        let remote = tempfile::tempdir().expect("temporary directory");
        git(
            remote.path(),
            &["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        let url = remote.path().to_str().expect("utf-8 path");
        git(path, &["remote", "add", "origin", url]);
        git(
            path,
            &["push", "--quiet", "--set-upstream", "origin", "main"],
        );
        remote
    }

    /// A second clone of `remote` that can push commits the first one lacks.
    pub(crate) fn clone_of(remote: &Path) -> TempDir {
        let directory = tempfile::tempdir().expect("temporary directory");
        let url = remote.to_str().expect("utf-8 path");
        git(directory.path(), &["clone", "--quiet", url, "."]);
        git(directory.path(), &["config", "user.name", "Other"]);
        git(
            directory.path(),
            &["config", "user.email", "other@example.com"],
        );
        git(directory.path(), &["config", "commit.gpgsign", "false"]);
        directory
    }

    pub(crate) fn write(path: &Path, relative: &str, content: impl AsRef<[u8]>) {
        let file = path.join(relative);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(file, content).expect("write file");
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::new_repository;
    use super::*;
    use std::fs;

    #[test]
    fn missing_git_binary_is_reported() {
        let error = run(&mut Command::new("/nonexistent/git"), &[0]).unwrap_err();
        assert!(matches!(error, GitError::GitNotFound), "{error:?}");
    }

    #[test]
    fn folder_outside_repository_is_reported() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let error = repository_root(directory.path()).unwrap_err();
        assert!(matches!(error, GitError::NotARepository), "{error:?}");
    }

    #[test]
    fn failing_command_keeps_stderr() {
        let repository = new_repository();
        let error = run(
            git_command(repository.path()).args(["rev-parse", "--verify", "missing-revision"]),
            &[0],
        )
        .unwrap_err();
        match error {
            GitError::CommandFailed { command, stderr } => {
                assert!(command.contains("rev-parse"), "{command}");
                assert!(!stderr.trim().is_empty());
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn subdirectory_resolves_to_repository_root() {
        let repository = new_repository();
        let subdirectory = repository.path().join("src/nested");
        fs::create_dir_all(&subdirectory).expect("create subdirectory");

        let root = repository_root(&subdirectory).expect("repository root");
        assert_eq!(
            root.canonicalize().expect("canonical root"),
            repository
                .path()
                .canonicalize()
                .expect("canonical repository"),
        );
    }
}
