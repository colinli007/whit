//! Changes made on disk by others (`file-watching`, and the refresh rule of
//! `worktree-diff`). Unit tests hand the watcher's batches in themselves.
use std::fs;

use super::*;
use crate::watcher::{Batch, Change};

impl Harness {
    /// What the watcher would report after these paths changed.
    fn changed(&self, changes: &[Change], cx: &mut TestAppContext) {
        let batch = Batch {
            changes: changes.iter().cloned().collect(),
            rescan: false,
        };
        let view = self.view.clone();
        self.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.files_changed(&batch, window, cx))
        });
        cx.run_until_parked();
    }

    fn file_text(&self, path: &str, cx: &mut TestAppContext) -> String {
        self.view.read_with(cx, |view, cx| {
            let buffer = view.editors.buffers[&*self.file_path(path)].buffer.read(cx);
            buffer.editor().expect("text").read(cx).value().to_string()
        })
    }

    fn file_deleted(&self, path: &str, cx: &mut TestAppContext) -> bool {
        self.view.read_with(cx, |view, cx| {
            view.editors.buffers[&*self.file_path(path)].buffer.read(cx).is_deleted()
        })
    }

    fn open_file(&self, path: &str, cx: &mut TestAppContext) {
        let view = self.view.clone();
        let key = self.file_tab(path);
        self.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.open_tab(key, false, window, cx))
        });
        cx.run_until_parked();
    }
}

fn worktree(path: &str) -> Change {
    Change::Worktree(path.into())
}

#[gpui_kit::test]
async fn a_change_on_disk_shows_without_switching_windows(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    write(repository.path(), "b.txt", "new\n");
    harness.changed(&[worktree("b.txt")], cx);
    harness.wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx).await;
}

#[gpui_kit::test]
async fn a_commit_in_the_terminal_clears_the_changes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    commit_all(repository.path());
    harness.changed(
        &[Change::Git("index".into()), Change::Git("refs/heads/main".into())],
        cx,
    );
    harness.wait_for(&[], cx).await;
}

#[gpui_kit::test]
async fn a_build_does_not_reread_the_changes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    write(path, ".gitignore", "target/\n");
    write(path, "target/old.o", "");
    git(path, &["add", ".gitignore"]);
    git(path, &["commit", "--quiet", "--message", "ignore"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    // A change nobody reported: a refresh would bring it in.
    write(path, "unreported.txt", "x\n");
    let build: Vec<Change> = (0..1000)
        .map(|ix| worktree(&format!("target/debug/{ix}.o")))
        .collect();
    harness.changed(&build, cx);
    cx.update(|cx| {
        assert_eq!(
            harness.loaded(cx),
            Some(vec![(Changes, "a.txt".to_owned())]),
            "the build did not read the changes again"
        )
    });

    harness.changed(&[worktree("unreported.txt")], cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "unreported.txt")], cx)
        .await;
}

#[gpui_kit::test]
async fn saving_brings_the_file_into_the_changes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    write(repository.path(), "README.md", "readme\n");
    commit_all(repository.path());
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[], cx).await;
    harness.open_file("README.md", cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });
    harness.press("cmd-s", cx);
    cx.run_until_parked();

    // The save's own write, as the watcher reports it.
    harness.changed(&[worktree("README.md")], cx);
    harness.wait_for(&[(Changes, "README.md")], cx).await;
    // It is not taken for someone else's change.
    assert_eq!(harness.file_text("README.md", cx), "xreadme\n");
}

#[gpui_kit::test]
async fn an_open_file_follows_the_disk_unless_edited(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx).await;
    harness.open_file("b.txt", cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.input("mine ", cx);
    });
    cx.run_until_parked();
    harness.open_file("a.txt", cx);

    write(repository.path(), "a.txt", "line\nafter\n");
    write(repository.path(), "b.txt", "theirs\n");
    harness.changed(&[worktree("a.txt"), worktree("b.txt")], cx);
    // Saved, `a.txt` reloads; `b.txt` keeps the edits in progress.
    assert_eq!(harness.file_text("a.txt", cx), "line\nafter\n");
    assert_eq!(harness.file_text("b.txt", cx), "mine after\n");

    fs::remove_file(repository.path().join("a.txt")).unwrap();
    harness.changed(&[worktree("a.txt")], cx);
    assert!(harness.file_deleted("a.txt", cx));
    assert!(harness.tabs(cx).contains(&harness.file_tab("a.txt")), "the tab stays");
}
