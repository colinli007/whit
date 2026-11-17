//! Branches, remote branches and tags, for the pickers that check out, create,
//! merge or delete them.
use std::path::Path;

use super::{GitError, git_command, run};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefKind {
    Local,
    /// A remote-tracking branch; `remote` is the first segment of its name.
    Remote {
        remote: String,
    },
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub kind: RefKind,
    /// The short name: `main`, `origin/main`, `v1.0`.
    pub name: String,
    /// The commit it points at; an annotated tag's own object is skipped.
    pub commit: String,
    pub author: String,
    /// Committer time in seconds since the epoch.
    pub date: Option<i64>,
    pub subject: String,
    /// A local branch's upstream, such as `origin/main`.
    pub upstream: Option<String>,
    /// A local branch's `(ahead, behind)` when it differs from its upstream,
    /// `(0, 0)` when the upstream is gone, and `None` when in step or without
    /// one — exactly when VS Code shows the counts.
    pub track: Option<(u32, u32)>,
}

impl Reference {
    /// The 7-character hash VS Code shows by default.
    pub fn short_commit(&self) -> &str {
        &self.commit[..self.commit.len().min(7)]
    }
}

/// Fields of one `for-each-ref` line. The `*` variants are an annotated tag's
/// target commit; for everything else they are empty.
const FORMAT: &str = "%(refname)%00%(objectname)%00%(*objectname)%00%(authorname)%00%(*authorname)%00%(committerdate:unix)%00%(*committerdate:unix)%00%(subject)%00%(*subject)%00%(upstream:short)%00%(upstream:track)";

/// Every local branch, remote branch and tag, most recently committed first,
/// as VS Code's default `git.branchSortOrder` lists them. `<remote>/HEAD` is
/// left out: it only names another remote branch.
pub fn list_references(root: &Path) -> Result<Vec<Reference>, GitError> {
    let output = run(
        git_command(root).args([
            "for-each-ref",
            "--sort=-committerdate",
            &format!("--format={FORMAT}"),
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ]),
        &[0],
    )?;
    Ok(String::from_utf8_lossy(&output)
        .lines()
        .filter_map(parse_reference)
        .collect())
}

fn parse_reference(line: &str) -> Option<Reference> {
    let fields: Vec<&str> = line.split('\0').collect();
    let [
        refname,
        object,
        tag_target,
        author,
        tag_author,
        date,
        tag_date,
        subject,
        tag_subject,
        upstream,
        track,
    ] = fields.as_slice()
    else {
        return None;
    };
    let either = |own: &str, target: &str| {
        if target.is_empty() {
            own.to_owned()
        } else {
            target.to_owned()
        }
    };
    let (kind, name) = if let Some(name) = refname.strip_prefix("refs/heads/") {
        (RefKind::Local, name)
    } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
        let (remote, branch) = name.split_once('/')?;
        if branch == "HEAD" {
            return None;
        }
        (
            RefKind::Remote {
                remote: remote.to_owned(),
            },
            name,
        )
    } else {
        (RefKind::Tag, refname.strip_prefix("refs/tags/")?)
    };
    let is_local = kind == RefKind::Local;
    Some(Reference {
        kind,
        name: name.to_owned(),
        commit: either(object, tag_target),
        author: either(author, tag_author),
        date: either(date, tag_date).parse().ok(),
        subject: either(subject, tag_subject),
        upstream: (is_local && !upstream.is_empty()).then(|| (*upstream).to_owned()),
        track: is_local.then(|| parse_track(track)).flatten(),
    })
}

/// Parses `[ahead 1, behind 2]`, `[ahead 1]`, `[behind 2]` or `[gone]`.
fn parse_track(track: &str) -> Option<(u32, u32)> {
    let inner = track.strip_prefix('[')?.strip_suffix(']')?;
    let (mut ahead, mut behind) = (0, 0);
    for part in inner.split(", ") {
        if let Some(count) = part.strip_prefix("ahead ") {
            ahead = count.parse().ok()?;
        } else if let Some(count) = part.strip_prefix("behind ") {
            behind = count.parse().ok()?;
        }
    }
    Some((ahead, behind))
}

/// Turns typed text into a name git accepts, as VS Code does: surrounding
/// whitespace and leading dashes go, and each character or sequence git
/// forbids in a ref name becomes `-`.
pub fn sanitize_branch_name(name: &str) -> String {
    let name = name.trim().trim_start_matches('-');
    let mut out = String::with_capacity(name.len());
    let chars: Vec<char> = name.chars().collect();
    let mut ix = 0;
    while ix < chars.len() {
        let rest: String = chars[ix..].iter().collect();
        let at_start = ix == 0;
        let (replaced, consumed) = if rest.starts_with("..") || rest.starts_with("/.") {
            (true, 2)
        } else if rest.starts_with(".lock/") {
            (true, 6)
        } else if rest == ".lock" {
            (true, 5)
        } else {
            let c = chars[ix];
            let last = ix + 1 == chars.len();
            let forbidden = matches!(c, '~' | '^' | ':' | '\\' | '*' | '[')
                || c.is_whitespace()
                || (at_start && c == '.')
                || (last && matches!(c, '/' | '.' | ']'));
            (forbidden, 1)
        };
        if replaced {
            out.push('-');
        } else {
            out.extend(&chars[ix..ix + consumed]);
        }
        ix += consumed;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{add_remote, clone_of, git, new_repository, write};
    use super::*;
    use std::process::Command;

    /// Commits everything at a fixed committer time, so sorting is decided.
    fn commit_at(path: &Path, message: &str, time: i64) {
        git(path, &["add", "--all"]);
        let status = Command::new("git")
            .current_dir(path)
            .env("GIT_COMMITTER_DATE", format!("@{time} +0000"))
            .env("GIT_AUTHOR_DATE", format!("@{time} +0000"))
            .args(["commit", "--quiet", "--message", message])
            .status()
            .expect("git should run");
        assert!(status.success());
    }

    fn names(references: &[Reference]) -> Vec<&str> {
        references.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn lists_branches_remote_branches_and_tags_newest_first() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        commit_at(path, "first", 1_000);
        git(path, &["tag", "light"]);
        git(path, &["tag", "-a", "annotated", "-m", "release"]);
        let remote = add_remote(path);
        git(path, &["checkout", "--quiet", "-b", "feature/x"]);
        write(path, "b.txt", "b\n");
        commit_at(path, "second", 2_000);
        git(path, &["checkout", "--quiet", "main"]);

        let references = list_references(path).unwrap();
        // A tag object has no committer date of its own, so git sorts an
        // annotated tag last, as VS Code shows it too.
        assert_eq!(
            names(&references),
            ["feature/x", "main", "origin/main", "light", "annotated"]
        );
        let feature = &references[0];
        assert_eq!(feature.kind, RefKind::Local);
        assert_eq!(feature.subject, "second");
        assert_eq!(feature.author, "Test");
        assert_eq!(feature.date, Some(2_000));
        assert_eq!(feature.upstream, None);
        assert_eq!(feature.track, None);

        let main = &references[1];
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        // In step with its upstream: VS Code shows no counts.
        assert_eq!(main.track, None);
        assert_eq!(
            references[2].kind,
            RefKind::Remote {
                remote: "origin".into()
            }
        );

        // An annotated tag reports the commit it tags, not the tag object.
        let annotated = &references[4];
        assert_eq!(annotated.kind, RefKind::Tag);
        assert_eq!(annotated.commit, main.commit);
        assert_eq!(annotated.subject, "first");
        assert_eq!(annotated.date, Some(1_000));
        assert_eq!(annotated.short_commit().len(), 7);

        // Ahead of and behind the upstream.
        let other = clone_of(remote.path());
        write(other.path(), "c.txt", "c\n");
        commit_at(other.path(), "remote", 3_000);
        git(other.path(), &["push", "--quiet"]);
        write(path, "d.txt", "d\n");
        commit_at(path, "local", 4_000);
        git(path, &["fetch", "--quiet"]);
        let main = list_references(path)
            .unwrap()
            .into_iter()
            .find(|r| r.name == "main")
            .unwrap();
        assert_eq!(main.track, Some((1, 1)));

        // A deleted upstream is gone. (The remote's default branch cannot be
        // deleted, so this uses another one.)
        git(
            path,
            &["push", "--quiet", "--set-upstream", "origin", "feature/x"],
        );
        git(
            path,
            &["push", "--quiet", "origin", "--delete", "feature/x"],
        );
        git(path, &["fetch", "--quiet", "--prune"]);
        let feature = list_references(path)
            .unwrap()
            .into_iter()
            .find(|r| r.name == "feature/x")
            .unwrap();
        assert_eq!(feature.track, Some((0, 0)));
    }

    #[test]
    fn remote_head_is_left_out() {
        let repository = new_repository();
        let path = repository.path();
        write(path, "a.txt", "a\n");
        commit_at(path, "first", 1_000);
        let remote = add_remote(path);
        let clone = clone_of(remote.path());
        let references = list_references(clone.path()).unwrap();
        assert_eq!(names(&references), ["main", "origin/main"]);
    }

    #[test]
    fn track_parsing() {
        assert_eq!(parse_track(""), None);
        assert_eq!(parse_track("[ahead 2]"), Some((2, 0)));
        assert_eq!(parse_track("[behind 15]"), Some((0, 15)));
        assert_eq!(parse_track("[ahead 1, behind 2]"), Some((1, 2)));
        assert_eq!(parse_track("[gone]"), Some((0, 0)));
    }

    #[test]
    fn branch_names_are_sanitized_like_vs_code() {
        assert_eq!(sanitize_branch_name("fix login bug"), "fix-login-bug");
        assert_eq!(sanitize_branch_name("  --feature "), "feature");
        assert_eq!(sanitize_branch_name("a..b"), "a-b");
        assert_eq!(sanitize_branch_name("a~b^c:d"), "a-b-c-d");
        assert_eq!(sanitize_branch_name("a\\b*c[d"), "a-b-c-d");
        assert_eq!(sanitize_branch_name(".hidden"), "-hidden");
        assert_eq!(sanitize_branch_name("feature/"), "feature-");
        assert_eq!(sanitize_branch_name("name."), "name-");
        assert_eq!(sanitize_branch_name("name.lock"), "name-");
        assert_eq!(sanitize_branch_name("a.lock/b"), "a-b");
        assert_eq!(sanitize_branch_name("a/.b"), "a-b");
        assert_eq!(sanitize_branch_name("end]"), "end-");
        assert_eq!(sanitize_branch_name("feature/login"), "feature/login");
        assert_eq!(sanitize_branch_name("中文分支"), "中文分支");
    }
}
