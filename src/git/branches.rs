//! Changes to branches: checking out, creating, renaming, deleting, merging
//! and rebasing, plus the stash steps a checkout over local changes needs.
//! Each runs the command VS Code 1.117 runs for it.
use std::path::Path;

use super::{
    GitError, run,
    write::{remote_command, write_command},
};

/// Checks out a local branch, or a tag as a detached HEAD.
pub fn checkout(root: &Path, name: &str) -> Result<(), GitError> {
    checkout_with(root, &[name])
}

/// Creates a local branch tracking `remote_branch` (`origin/x`) and checks it out.
pub fn checkout_tracking(root: &Path, remote_branch: &str) -> Result<(), GitError> {
    checkout_with(root, &["--track", remote_branch])
}

/// Checks out `commit` as a detached HEAD.
pub fn checkout_detached(root: &Path, commit: &str) -> Result<(), GitError> {
    checkout_with(root, &["--detach", commit])
}

/// Creates `name` at `start` without an upstream and checks it out.
pub fn create_branch(root: &Path, name: &str, start: &str) -> Result<(), GitError> {
    checkout_with(root, &["-b", name, "--no-track", start])
}

fn checkout_with(root: &Path, arguments: &[&str]) -> Result<(), GitError> {
    match run(
        write_command(root).args(["checkout", "-q"]).args(arguments),
        &[0],
    ) {
        Err(GitError::CommandFailed { stderr, .. }) if would_overwrite(&stderr) => {
            Err(GitError::DirtyWorkTree)
        }
        result => result.map(drop),
    }
}

/// Git's refusal to check out over local changes it would lose.
fn would_overwrite(stderr: &str) -> bool {
    stderr.contains("Please commit your changes or stash them")
        || stderr.contains("Please, commit your changes or stash them")
}

/// Renames the current branch.
pub fn rename_branch(root: &Path, name: &str) -> Result<(), GitError> {
    run(write_command(root).args(["branch", "-m", name]), &[0]).map(drop)
}

/// Deletes a local branch; without `force`, git refuses one whose commits
/// would be lost.
pub fn delete_branch(root: &Path, name: &str, force: bool) -> Result<(), GitError> {
    let flag = if force { "-D" } else { "-d" };
    match run(write_command(root).args(["branch", flag, name]), &[0]) {
        Err(GitError::CommandFailed { stderr, .. }) if stderr.contains("is not fully merged") => {
            Err(GitError::BranchNotFullyMerged)
        }
        result => result.map(drop),
    }
}

/// Deletes `branch` on `remote`.
pub fn delete_remote_branch(root: &Path, remote: &str, branch: &str) -> Result<(), GitError> {
    run(
        remote_command(root).args(["push", remote, "--delete", branch]),
        &[0],
    )
    .map(drop)
}

/// Merges `reference` into the current branch.
pub fn merge(root: &Path, reference: &str) -> Result<(), GitError> {
    stopping_on_conflict(root, write_command(root).args(["merge", reference]))
}

/// Rebases the current branch onto `reference`.
pub fn rebase(root: &Path, reference: &str) -> Result<(), GitError> {
    stopping_on_conflict(root, write_command(root).args(["rebase", reference]))
}

/// Continues a stopped rebase with the message git prepared for each commit.
pub fn rebase_continue(root: &Path) -> Result<(), GitError> {
    stopping_on_conflict(
        root,
        write_command(root)
            .env("GIT_EDITOR", "true")
            .args(["rebase", "--continue"]),
    )
}

pub fn merge_abort(root: &Path) -> Result<(), GitError> {
    run(write_command(root).args(["merge", "--abort"]), &[0]).map(drop)
}

pub fn rebase_abort(root: &Path) -> Result<(), GitError> {
    run(write_command(root).args(["rebase", "--abort"]), &[0]).map(drop)
}

/// Stashes every change, untracked files included.
pub fn stash_all(root: &Path) -> Result<(), GitError> {
    run(
        write_command(root).args(["stash", "push", "--include-untracked"]),
        &[0],
    )
    .map(drop)
}

/// Applies and drops the latest stash.
pub fn stash_pop(root: &Path) -> Result<(), GitError> {
    stopping_on_conflict(root, write_command(root).args(["stash", "pop"]))
}

/// Throws away every change: the index and working tree return to HEAD and
/// untracked files are deleted. Ignored files stay.
pub fn discard_everything(root: &Path) -> Result<(), GitError> {
    run(write_command(root).args(["reset", "--hard", "-q"]), &[0])?;
    run(write_command(root).args(["clean", "-fdq"]), &[0]).map(drop)
}

/// Runs a command that may stop for conflicts; a failure that left unmerged
/// paths is a conflict, whatever words git used.
fn stopping_on_conflict(root: &Path, command: &mut std::process::Command) -> Result<(), GitError> {
    match run(command, &[0]) {
        Err(error) if has_unmerged_paths(root)? => {
            log::info!("stopped for conflicts: {error}");
            Err(GitError::Conflict)
        }
        result => result.map(drop),
    }
}

fn has_unmerged_paths(root: &Path) -> Result<bool, GitError> {
    let output = run(write_command(root).args(["ls-files", "--unmerged"]), &[0])?;
    Ok(!output.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{
        add_remote, clone_of, commit_all, git, git_output, new_repository, write,
    };
    use super::super::{InProgress, list_references, load_snapshot};
    use super::*;
    use std::fs;

    fn head(path: &Path) -> String {
        git_output(path, &["rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_owned()
    }

    fn status(path: &Path) -> String {
        git_output(path, &["status", "--porcelain"])
    }

    /// A repository on `main` with a second branch `dev` that changed `a.txt`.
    fn two_branches() -> tempfile::TempDir {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "main\n");
        write(path, "b.txt", "same\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "-b", "dev"]);
        write(path, "a.txt", "dev\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "main"]);
        repository
    }

    #[test]
    fn checking_out_branches_tags_and_detached_commits() {
        let repository = two_branches();
        let path = repository.path();
        checkout(path, "dev").unwrap();
        assert_eq!(head(path), "dev");

        git(path, &["tag", "v1", "main"]);
        checkout(path, "v1").unwrap();
        assert_eq!(head(path), "HEAD");

        let dev = git_output(path, &["rev-parse", "dev"]);
        checkout_detached(path, dev.trim()).unwrap();
        assert_eq!(head(path), "HEAD");
        assert_eq!(git_output(path, &["rev-parse", "HEAD"]), dev);
    }

    #[test]
    fn checking_out_a_remote_branch_tracks_it() {
        let repository = two_branches();
        let path = repository.path();
        let remote = add_remote(path);
        git(path, &["push", "--quiet", "origin", "dev"]);
        let clone = clone_of(remote.path());
        checkout_tracking(clone.path(), "origin/dev").unwrap();
        assert_eq!(head(clone.path()), "dev");
        let snapshot = load_snapshot(clone.path(), None).unwrap().unwrap();
        assert_eq!(
            snapshot.branch.upstream.map(|upstream| upstream.name),
            Some("origin/dev".to_owned())
        );
    }

    #[test]
    fn creating_a_branch_from_a_start_point() {
        let repository = two_branches();
        let path = repository.path();
        create_branch(path, "hotfix", "dev").unwrap();
        assert_eq!(head(path), "hotfix");
        assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "dev\n");
        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        assert_eq!(snapshot.branch.upstream, None);
    }

    #[test]
    fn local_changes_that_would_be_lost_stop_the_checkout() {
        let repository = two_branches();
        let path = repository.path();
        write(path, "a.txt", "edited\n");
        assert!(matches!(
            checkout(path, "dev"),
            Err(GitError::DirtyWorkTree)
        ));
        assert_eq!(head(path), "main");

        // A change git can carry over does not stop it.
        git(path, &["checkout", "--", "a.txt"]);
        write(path, "b.txt", "edited\n");
        checkout(path, "dev").unwrap();
        assert_eq!(head(path), "dev");
        assert_eq!(fs::read_to_string(path.join("b.txt")).unwrap(), "edited\n");
    }

    #[test]
    fn stash_pop_and_discard_everything() {
        let repository = two_branches();
        let path = repository.path();
        write(path, "a.txt", "edited\n");
        write(path, "new.txt", "new\n");
        stash_all(path).unwrap();
        assert_eq!(status(path), "");

        // Migrating the change to `dev` conflicts with its own `a.txt`.
        checkout(path, "dev").unwrap();
        assert!(matches!(stash_pop(path), Err(GitError::Conflict)));
        assert!(path.join("new.txt").exists());

        discard_everything(path).unwrap();
        assert_eq!(status(path), "");
        assert!(!path.join("new.txt").exists());

        // Without a conflict the stash comes back whole and is dropped.
        git(path, &["stash", "drop", "--quiet"]);
        write(path, "b.txt", "edited\n");
        write(path, "new.txt", "new\n");
        stash_all(path).unwrap();
        stash_pop(path).unwrap();
        assert_eq!(fs::read_to_string(path.join("b.txt")).unwrap(), "edited\n");
        assert!(path.join("new.txt").exists());
        assert_eq!(git_output(path, &["stash", "list"]), "");
    }

    #[test]
    fn renaming_and_deleting_branches() {
        let repository = two_branches();
        let path = repository.path();
        checkout(path, "dev").unwrap();
        rename_branch(path, "feature").unwrap();
        assert_eq!(head(path), "feature");
        checkout(path, "main").unwrap();

        // `feature` has a commit `main` lacks.
        assert!(matches!(
            delete_branch(path, "feature", false),
            Err(GitError::BranchNotFullyMerged)
        ));
        delete_branch(path, "feature", true).unwrap();
        let names: Vec<String> = list_references(path)
            .unwrap()
            .into_iter()
            .map(|reference| reference.name)
            .collect();
        assert_eq!(names, ["main"]);
    }

    #[test]
    fn deleting_a_remote_branch() {
        let repository = two_branches();
        let path = repository.path();
        let remote = add_remote(path);
        git(path, &["push", "--quiet", "origin", "dev"]);
        delete_remote_branch(path, "origin", "dev").unwrap();
        let remote_heads = git_output(remote.path(), &["branch", "--list"]);
        assert!(!remote_heads.contains("dev"), "{remote_heads}");
    }

    #[test]
    fn merging_with_and_without_conflicts() {
        let repository = two_branches();
        let path = repository.path();
        // Fast-forward-free merge of a branch that does not conflict.
        git(path, &["checkout", "--quiet", "-b", "side"]);
        write(path, "c.txt", "c\n");
        commit_all(path);
        git(path, &["checkout", "--quiet", "main"]);
        write(path, "d.txt", "d\n");
        commit_all(path);
        merge(path, "side").unwrap();
        assert!(path.join("c.txt").exists());

        // `main` and `dev` both changed `a.txt`.
        write(path, "a.txt", "main again\n");
        commit_all(path);
        assert!(matches!(merge(path, "dev"), Err(GitError::Conflict)));
        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        assert_eq!(snapshot.in_progress, Some(InProgress::Merge));

        // Resolving, staging and committing concludes the merge.
        write(path, "a.txt", "resolved\n");
        git(path, &["add", "a.txt"]);
        let message = load_snapshot(path, None)
            .unwrap()
            .unwrap()
            .merge_message
            .expect("a merge message");
        crate::git::commit(path, &message, false).unwrap();
        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        assert_eq!(snapshot.in_progress, None);
        let parents = git_output(path, &["log", "-1", "--format=%P"]);
        assert_eq!(parents.split_whitespace().count(), 2);

        // Aborting returns to before the merge.
        git(path, &["checkout", "--quiet", "dev"]);
        write(path, "a.txt", "dev again\n");
        commit_all(path);
        assert!(matches!(merge(path, "main"), Err(GitError::Conflict)));
        merge_abort(path).unwrap();
        assert_eq!(status(path), "");
    }

    #[test]
    fn rebasing_continuing_and_aborting() {
        let repository = two_branches();
        let path = repository.path();
        write(path, "a.txt", "main again\n");
        commit_all(path);
        checkout(path, "dev").unwrap();
        let before = git_output(path, &["rev-parse", "HEAD"]);

        assert!(matches!(rebase(path, "main"), Err(GitError::Conflict)));
        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        assert_eq!(snapshot.in_progress, Some(InProgress::Rebase));
        rebase_abort(path).unwrap();
        assert_eq!(git_output(path, &["rev-parse", "HEAD"]), before);

        assert!(matches!(rebase(path, "main"), Err(GitError::Conflict)));
        write(path, "a.txt", "resolved\n");
        git(path, &["add", "a.txt"]);
        rebase_continue(path).unwrap();
        let snapshot = load_snapshot(path, None).unwrap().unwrap();
        assert_eq!(snapshot.in_progress, None);
        assert_eq!(head(path), "dev");
        assert_eq!(
            fs::read_to_string(path.join("a.txt")).unwrap(),
            "resolved\n"
        );

        // A rebase without conflicts just succeeds.
        git(path, &["checkout", "--quiet", "-b", "clean", "main"]);
        write(path, "z.txt", "z\n");
        commit_all(path);
        rebase(path, "dev").unwrap();
    }
}
