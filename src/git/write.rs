//! Changes to the repository: the index, commits and remotes. These run git
//! with the user's own configuration, hooks and signing, exactly as the same
//! command typed in a terminal would.
use std::{
    fs,
    io::{self, Write as _},
    os::unix::process::CommandExt as _,
    path::Path,
    process::{Command, Stdio},
};

use super::{GitError, base_command, finish, run, spawn_failed};

/// A command that changes the repository. Unlike reading, it takes the locks it
/// needs and lets fsmonitor run as configured.
pub(super) fn write_command(root: &Path) -> Command {
    base_command(root)
}

/// A command that talks to a remote. It runs without a controlling terminal:
/// `GIT_TERMINAL_PROMPT=0` only stops git's own prompts, while ssh asks for
/// passphrases and host keys on `/dev/tty` directly and would wait there
/// forever. Detached, ssh fails at once unless the user's agent, keychain or
/// credential helper can answer without asking.
pub(super) fn remote_command(root: &Path) -> Command {
    let mut command = write_command(root);
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the
    // forked child, which is all `pre_exec` permits.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
}

/// Paths are literal names, never pathspec patterns.
fn with_paths<'a>(command: &'a mut Command, paths: &[String]) -> &'a mut Command {
    command.arg("--").args(paths)
}

/// Stages the given paths, deletions included.
pub fn stage(root: &Path, paths: &[String]) -> Result<(), GitError> {
    run(
        with_paths(
            write_command(root).args(["--literal-pathspecs", "add", "--all"]),
            paths,
        ),
        &[0],
    )
    .map(drop)
}

/// Stages every change, untracked files included.
pub fn stage_all(root: &Path) -> Result<(), GitError> {
    run(write_command(root).args(["add", "--all"]), &[0]).map(drop)
}

/// Returns the given paths' index entries to HEAD, or removes them from the
/// index before the first commit. The working tree is left alone.
pub fn unstage(root: &Path, paths: &[String], has_commits: bool) -> Result<(), GitError> {
    let mut command = write_command(root);
    command.arg("--literal-pathspecs");
    if has_commits {
        command.args(["restore", "--staged"]);
    } else {
        command.args(["rm", "--cached", "-r", "-q"]);
    }
    run(with_paths(&mut command, paths), &[0]).map(drop)
}

/// Returns tracked files to their index contents and deletes untracked ones.
/// Staged changes are kept.
pub fn discard(root: &Path, tracked: &[String], untracked: &[String]) -> Result<(), GitError> {
    if !tracked.is_empty() {
        run(
            with_paths(
                write_command(root).args(["--literal-pathspecs", "restore", "--worktree"]),
                tracked,
            ),
            &[0],
        )?;
    }
    // Only the listed files: `git clean` would also take whatever else an
    // untracked directory holds.
    for path in untracked {
        fs::remove_file(root.join(path)).map_err(GitError::Io)?;
    }
    Ok(())
}

/// Commits the index. Amending with an empty message keeps the previous one.
pub fn commit(root: &Path, message: &str, amend: bool) -> Result<(), GitError> {
    let mut command = write_command(root);
    command.arg("commit");
    if amend {
        command.arg("--amend");
        if message.trim().is_empty() {
            return run(command.arg("--no-edit"), &[0]).map(drop);
        }
    }
    // Through stdin, so no quoting or length limit applies to the message.
    command
        .args(["--file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(spawn_failed)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(message.as_bytes()).map_err(GitError::Io)?;
    }
    let output = child.wait_with_output().map_err(GitError::Io)?;
    finish(&command, output, &[0]).map(drop)
}

/// `git pull`, following the user's configuration such as `pull.rebase`.
pub fn pull(root: &Path) -> Result<(), GitError> {
    run(remote_command(root).arg("pull"), &[0]).map(drop)
}

/// Pushes the current branch to its upstream.
pub fn push(root: &Path) -> Result<(), GitError> {
    run(remote_command(root).arg("push"), &[0]).map(drop)
}

/// Pushes `branch`, which has no upstream yet, to `remote` and makes the
/// remote branch its upstream.
pub fn publish(root: &Path, remote: &str, branch: &str) -> Result<(), GitError> {
    run(
        remote_command(root).args(["push", "--set-upstream", remote, branch]),
        &[0],
    )
    .map(drop)
}

/// Updates remote-tracking branches without touching the working tree.
pub fn fetch(root: &Path) -> Result<(), GitError> {
    run(remote_command(root).arg("fetch"), &[0]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{
        add_remote, clone_of, commit_all, git, git_output, new_repository, write,
    };
    use super::super::{load_snapshot, read::ChangeGroup, remotes};
    use super::*;
    use std::time::{Duration, Instant};

    /// Group, path and letter of every changed file.
    fn groups(root: &Path) -> Vec<(ChangeGroup, String, &'static str)> {
        load_snapshot(root, None)
            .expect("load snapshot")
            .expect("a snapshot")
            .files
            .iter()
            .map(|changed| {
                (
                    changed.group,
                    changed.file.path().to_string(),
                    changed.letter.as_str(),
                )
            })
            .collect()
    }

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    use ChangeGroup::{Changes, Staged};

    #[test]
    fn staging_and_unstaging_move_files_between_groups() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        write(path, "gone.txt", "g\n");
        commit_all(path);
        write(path, "a.txt", "changed\n");
        fs::remove_file(path.join("gone.txt")).expect("delete");
        write(path, "new [1].txt", "n\n");

        stage(path, &paths(&["a.txt", "gone.txt", "new [1].txt"])).expect("stage");
        assert_eq!(
            groups(path),
            vec![
                (Staged, "a.txt".into(), "M"),
                (Staged, "gone.txt".into(), "D"),
                (Staged, "new [1].txt".into(), "A"),
            ]
        );
        assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "changed\n");

        unstage(path, &paths(&["a.txt", "new [1].txt"]), true).expect("unstage");
        assert_eq!(
            groups(path),
            vec![
                (Staged, "gone.txt".into(), "D"),
                (Changes, "a.txt".into(), "M"),
                (Changes, "new [1].txt".into(), "U"),
            ]
        );
    }

    #[test]
    fn unstaging_before_the_first_commit() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        stage_all(path).expect("stage all");
        assert_eq!(groups(path), vec![(Staged, "a.txt".into(), "A")]);

        unstage(path, &paths(&["a.txt"]), false).expect("unstage");
        assert_eq!(groups(path), vec![(Changes, "a.txt".into(), "U")]);
    }

    #[test]
    fn discarding_keeps_staged_changes_and_deletes_untracked_files() {
        let repository = new_repository();
        let path = repository.path();
        let lines: String = (1..=10).map(|line| format!("{line}\n")).collect();
        write(path, "a.txt", &lines);
        commit_all(path);
        let staged = lines.replace("1\n", "one\n");
        write(path, "a.txt", &staged);
        git(path, &["add", "a.txt"]);
        write(path, "a.txt", staged.replace("10\n", "ten\n"));
        write(path, "dir/tmp.txt", "t\n");
        write(path, "dir/keep.txt", "k\n");

        discard(path, &paths(&["a.txt"]), &paths(&["dir/tmp.txt"])).expect("discard");
        assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), staged);
        assert!(!path.join("dir/tmp.txt").exists());
        assert!(path.join("dir/keep.txt").exists());
        assert_eq!(
            groups(path),
            vec![
                (Staged, "a.txt".into(), "M"),
                (Changes, "dir/keep.txt".into(), "U"),
            ]
        );
    }

    #[test]
    fn commit_message_is_kept_verbatim() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        stage_all(path).expect("stage");
        let message = "修复 \"quoted\" $HOME `x`\n\nsecond paragraph";

        commit(path, message, false).expect("commit");
        assert_eq!(
            git_output(path, &["log", "-1", "--format=%B"]).trim_end(),
            message
        );
    }

    #[test]
    fn commit_contains_only_the_index() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        write(path, "b.txt", "b\n");
        commit_all(path);
        write(path, "a.txt", "a2\n");
        write(path, "b.txt", "b2\n");
        stage(path, &paths(&["a.txt"])).expect("stage");

        commit(path, "fix", false).expect("commit");
        assert_eq!(
            git_output(path, &["show", "--name-only", "--format=", "HEAD"]).trim(),
            "a.txt"
        );
        assert_eq!(groups(path), vec![(Changes, "b.txt".into(), "M")]);
    }

    #[test]
    fn amending_keeps_or_replaces_the_message() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        stage_all(path).expect("stage");
        commit(path, "feat", false).expect("commit");

        write(path, "c.txt", "c\n");
        stage_all(path).expect("stage");
        commit(path, "", true).expect("amend");
        assert_eq!(
            git_output(path, &["rev-list", "--count", "HEAD"]).trim(),
            "1"
        );
        assert_eq!(
            git_output(path, &["log", "-1", "--format=%s"]).trim(),
            "feat"
        );
        assert_eq!(
            git_output(path, &["show", "--name-only", "--format=", "HEAD"]).trim(),
            "a.txt\nc.txt"
        );

        commit(path, "feat: better", true).expect("amend");
        assert_eq!(
            git_output(path, &["log", "-1", "--format=%s"]).trim(),
            "feat: better"
        );
    }

    #[test]
    fn a_failing_hook_reports_its_output() {
        let repository = new_repository();
        let path = repository.path();
        git(path, &["config", "core.hooksPath", ".git/hooks"]);
        write(
            path,
            ".git/hooks/pre-commit",
            "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n",
        );
        std::process::Command::new("chmod")
            .args(["+x", ".git/hooks/pre-commit"])
            .current_dir(path)
            .status()
            .expect("chmod");
        write(path, "a.txt", "a\n");
        stage_all(path).expect("stage");

        let error = commit(path, "blocked", false).unwrap_err();
        assert!(error.to_string().contains("hook says no"), "{error}");
        assert!(!error.needs_authentication());
    }

    #[test]
    fn nothing_staged_is_reported() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        commit_all(path);

        let error = commit(path, "empty", false).unwrap_err();
        assert!(error.to_string().contains("nothing"), "{error}");
    }

    #[test]
    fn push_pull_and_publish_with_a_local_remote() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        commit_all(path);
        assert!(remotes(path).unwrap().is_empty());

        let remote = add_remote(path);
        let found = remotes(path).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "origin");
        assert_eq!(found[0].push_url, remote.path().to_str().unwrap());
        write(path, "a.txt", "local\n");
        commit_all(path);
        push(path).expect("push");
        let other = clone_of(remote.path());
        assert_eq!(
            fs::read_to_string(other.path().join("a.txt")).unwrap(),
            "local\n"
        );

        write(other.path(), "b.txt", "remote\n");
        commit_all(other.path());
        git(other.path(), &["push", "--quiet"]);
        fetch(path).expect("fetch");
        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        assert_eq!(
            snapshot.branch.upstream.map(|upstream| upstream.behind),
            Some(1)
        );
        pull(path).expect("pull");
        assert!(path.join("b.txt").exists());

        git(path, &["checkout", "--quiet", "-b", "feature"]);
        publish(path, "origin", "feature").expect("publish");
        let upstream = load_snapshot(path, None).unwrap().unwrap().branch.upstream;
        assert_eq!(
            upstream.map(|upstream| upstream.name),
            Some("origin/feature".into())
        );
    }

    #[test]
    fn ssh_cannot_wait_on_the_terminal() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        commit_all(path);
        // An ssh that asks on the terminal, as ssh does for a passphrase.
        git(
            path,
            &[
                "config",
                "core.sshCommand",
                "sh -c 'exec 3</dev/tty; read answer <&3'",
            ],
        );
        git(
            path,
            &["remote", "add", "origin", "ssh://example.invalid/repo.git"],
        );
        git(path, &["config", "branch.main.remote", "origin"]);
        git(path, &["config", "branch.main.merge", "refs/heads/main"]);

        let started = Instant::now();
        assert!(push(path).is_err());
        assert!(fetch(path).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn credential_prompts_are_recognized() {
        let error = GitError::CommandFailed {
            command: "git push".into(),
            stderr: "fatal: could not read Username for 'https://example.com': terminal prompts disabled".into(),
        };
        assert!(error.needs_authentication());
    }
}
