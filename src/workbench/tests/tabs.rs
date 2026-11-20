//! The tabs of the editor area (`file-editor`, and the tab rules of
//! `worktree-diff` and `workbench-layout`).
use std::fs;

use gpui_kit::component::input::{EditorState, Position};

use super::*;
use crate::workbench::editor::UNSUPPORTED_FILE;

impl Harness {
    fn open_tab(&self, key: TabKey, preview: bool, cx: &mut TestAppContext) {
        let view = self.view.clone();
        self.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.open_tab(key, preview, window, cx))
        });
        cx.run_until_parked();
    }

    fn is_preview(&self, key: &TabKey, cx: &mut TestAppContext) -> bool {
        self.view.read_with(cx, |view, _| {
            view.editors.group.is_preview(key)
        })
    }

    fn double_click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.double_click(id, cx);
        });
        cx.run_until_parked();
    }

    /// Types into the active file tab's editor.
    fn type_in_file(&self, path: &str, text: &str, cx: &mut TestAppContext) {
        let editor = self.view.read_with(cx, |view, cx| {
            view.editors.buffers[&*self.file_path(path)].buffer.read(cx).editor().cloned().expect("text")
        });
        self.update(cx, |window, cx| {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            window.render_frame(cx);
            window.input(text, cx);
        });
        cx.run_until_parked();
    }

    fn close_button(&self, key: &TabKey, cx: &mut TestAppContext) {
        let id = tab_id(key);
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.hover(id.clone(), cx);
            window.render_frame(cx);
            window.click(SharedString::from(format!("{id}-close")), cx);
        });
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
async fn a_click_previews_and_a_double_click_keeps(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt", "c.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt"), (Changes, "c.txt")], cx)
        .await;
    // The first file opens as a preview with the repository.
    assert_eq!(harness.tabs(cx), [diff_tab(Changes, "a.txt")]);
    assert!(harness.is_preview(&diff_tab(Changes, "a.txt"), cx));

    // Clicks replace the preview in place.
    harness.click(file_row_id(Changes, "b.txt"), cx);
    assert_eq!(harness.tabs(cx), [diff_tab(Changes, "b.txt")]);
    assert_eq!(harness.open(cx), (Some((Changes, "b.txt".into())), vec!["b.txt".into()]));

    // A double click keeps the tab; the next click opens a new preview.
    harness.double_click(file_row_id(Changes, "b.txt"), cx);
    assert!(!harness.is_preview(&diff_tab(Changes, "b.txt"), cx));
    harness.click(file_row_id(Changes, "c.txt"), cx);
    assert_eq!(
        harness.tabs(cx),
        [diff_tab(Changes, "b.txt"), diff_tab(Changes, "c.txt")]
    );
    assert!(harness.is_preview(&diff_tab(Changes, "c.txt"), cx));

    // Double-clicking a preview tab keeps it too.
    harness.double_click(tab_id(&diff_tab(Changes, "c.txt")), cx);
    assert!(!harness.is_preview(&diff_tab(Changes, "c.txt"), cx));
}

#[gpui_kit::test]
async fn the_open_file_button_opens_an_editable_tab(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    harness.click_row_action(file_row_id(Changes, "a.txt"), "open-changes-a.txt", cx);
    cx.run_until_parked();
    // The diff tab stays; the file tab opens beside it, kept.
    assert_eq!(
        harness.tabs(cx),
        [diff_tab(Changes, "a.txt"), harness.file_tab("a.txt")]
    );
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("a.txt")));
    assert!(!harness.is_preview(&harness.file_tab("a.txt"), cx));
    // A file tab has no diff buttons, and the breadcrumbs show its path.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("toggle-inline").is_none());
        assert_eq!(window.find("breadcrumbs").label(), Some("a.txt"));
    });

    // Opening it again reuses the tab.
    harness.click_row_action(file_row_id(Changes, "a.txt"), "open-changes-a.txt", cx);
    assert_eq!(harness.tabs(cx).len(), 2);
}

#[gpui_kit::test]
async fn the_diff_tab_opens_its_file_from_the_tab_bar(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "gone.txt"]);
    fs::remove_file(repository.path().join("gone.txt")).unwrap();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt"), (Changes, "gone.txt")], cx).await;

    harness.click("open-file", cx);
    cx.run_until_parked();
    // The file tab opens beside the diff, kept and editable.
    assert_eq!(
        harness.tabs(cx),
        [diff_tab(Changes, "a.txt"), harness.file_tab("a.txt")]
    );
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("a.txt")));
    assert!(!harness.is_preview(&harness.file_tab("a.txt"), cx));
    // A file tab has no button to open itself.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("open-file").is_none());
    });

    // A deleted file has nothing to open.
    harness.click(file_row_id(Changes, "gone.txt"), cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("open-file").is_none());
        assert!(window.try_find("toggle-inline").is_some());
    });
}

#[gpui_kit::test]
async fn editing_a_preview_keeps_it(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx).await;
    harness.click(tab_id(&diff_tab(Changes, "a.txt")), cx);
    let close = diff_tab(Changes, "a.txt");
    harness.close_button(&close, cx);

    harness.open_tab(harness.file_tab("a.txt"), true, cx);
    assert!(harness.is_preview(&harness.file_tab("a.txt"), cx));
    harness.type_in_file("a.txt", "x", cx);
    assert!(!harness.is_preview(&harness.file_tab("a.txt"), cx));

    harness.open_tab(harness.file_tab("b.txt"), true, cx);
    assert_eq!(harness.tabs(cx), [harness.file_tab("a.txt"), harness.file_tab("b.txt")]);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        // The dot of unsaved edits, on a tab the pointer is not over.
        assert!(window.try_find(format!("{}-dirty", tab_id(&harness.file_tab("a.txt")))).is_some());
    });
}

#[gpui_kit::test]
async fn closing_returns_to_the_most_recently_used_tab(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt", "c.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt"), (Changes, "c.txt")], cx)
        .await;
    for name in ["a.txt", "b.txt", "c.txt"] {
        harness.open_tab(harness.file_tab(name), false, cx);
    }
    harness.click(tab_id(&harness.file_tab("a.txt")), cx);
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("a.txt")));

    harness.close_button(&harness.file_tab("a.txt"), cx);
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("c.txt")));
}

#[gpui_kit::test]
async fn files_of_the_same_name_show_their_folders(cx: &mut TestAppContext) {
    let repository = new_repository();
    let path = repository.path();
    write(path, "src/mod.rs", "a\n");
    write(path, "src/git/mod.rs", "b\n");
    commit_all(path);
    let harness = open_window(Some(path.to_path_buf()), cx);
    cx.run_until_parked();
    harness.open_tab(harness.file_tab("src/mod.rs"), false, cx);
    harness.open_tab(harness.file_tab("src/git/mod.rs"), false, cx);

    for (key, folder) in [("src/mod.rs", "src"), ("src/git/mod.rs", "src/git")] {
        let id = format!("{}-folder", tab_id(&harness.file_tab(key)));
        assert_eq!(harness.label(SharedString::from(id), cx).as_deref(), Some(folder));
    }
}

#[gpui_kit::test]
async fn a_binary_file_shows_why_it_is_not_shown(cx: &mut TestAppContext) {
    let repository = new_repository();
    let path = repository.path();
    write(path, "logo.png", b"\x89PNG\r\n\x1a\n\0\0\0\r");
    commit_all(path);
    let harness = open_window(Some(path.to_path_buf()), cx);
    cx.run_until_parked();

    harness.open_tab(harness.file_tab("logo.png"), true, cx);
    assert_eq!(harness.label("file-message", cx).as_deref(), Some(UNSUPPORTED_FILE));
}

#[gpui_kit::test]
async fn staging_closes_the_diff_tab_and_keeps_the_others(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "c.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt"), (Changes, "c.txt")], cx).await;
    harness.open_tab(harness.file_tab("c.txt"), false, cx);
    harness.click(tab_id(&diff_tab(Changes, "a.txt")), cx);

    harness.click_row_action(file_row_id(Changes, "a.txt"), "stage-changes-a.txt", cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Staged, "a.txt"), (Changes, "c.txt")], cx).await;
    assert_eq!(harness.tabs(cx), [harness.file_tab("c.txt")]);
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("c.txt")));
}

impl Harness {
    fn file_editor(&self, path: &str, cx: &mut TestAppContext) -> Entity<EditorState> {
        self.view.read_with(cx, |view, cx| {
            view.editors.buffers[&*self.file_path(path)].buffer.read(cx).editor().cloned().expect("text")
        })
    }
}

#[gpui_kit::test]
async fn the_status_bar_shows_the_cursor_of_a_file_tab(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    write(repository.path(), "a.txt", "one\ntwo\n");
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.open_tab(harness.file_tab("a.txt"), false, cx);

    let editor = harness.file_editor("a.txt", cx);
    harness.update(cx, |window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(1, 2), window, cx)
        });
    });
    cx.run_until_parked();
    assert_eq!(harness.label("status-cursor", cx).as_deref(), Some("行 2，列 3"));

    harness.click(tab_id(&diff_tab(Changes, "a.txt")), cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("status-cursor").is_none());
    });
}

#[gpui_kit::test]
async fn cmd_s_saves_and_cmd_w_closes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.open_tab(harness.file_tab("a.txt"), false, cx);
    harness.type_in_file("a.txt", "x", cx);

    harness.press("cmd-s", cx);
    cx.run_until_parked();
    assert_eq!(fs::read_to_string(repository.path().join("a.txt")).unwrap(), "xafter\n");
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(format!("{}-dirty", tab_id(&harness.file_tab("a.txt")))).is_none());
    });

    harness.press("cmd-w", cx);
    cx.run_until_parked();
    assert_eq!(harness.tabs(cx), [diff_tab(Changes, "a.txt")]);
    // ⌘S on a diff tab does nothing.
    harness.press("cmd-s", cx);
    cx.run_until_parked();
    assert_eq!(harness.notification_count(cx), 0);
}

#[gpui_kit::test]
async fn a_conflicting_save_offers_to_overwrite(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.open_tab(harness.file_tab("a.txt"), false, cx);
    harness.type_in_file("a.txt", "x", cx);
    write(repository.path(), "a.txt", "theirs, longer\n");

    harness.press("cmd-s", cx);
    cx.run_until_parked();
    assert_eq!(harness.notification_count(cx), 1);
    assert_eq!(
        fs::read_to_string(repository.path().join("a.txt")).unwrap(),
        "theirs, longer\n"
    );

    // Past the notification's slide-in, which runs on the simulated clock.
    cx.executor().advance_clock(Duration::from_secs(1));
    harness.click("save-overwrite", cx);
    cx.run_until_parked();
    assert_eq!(fs::read_to_string(repository.path().join("a.txt")).unwrap(), "xafter\n");
}

#[gpui_kit::test]
async fn a_failed_save_is_reported(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.open_tab(harness.file_tab("a.txt"), false, cx);
    let file = repository.path().join("a.txt");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    harness.type_in_file("a.txt", "x", cx);

    harness.press("cmd-s", cx);
    cx.run_until_parked();
    assert_eq!(harness.notification_count(cx), 1);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(format!("{}-dirty", tab_id(&harness.file_tab("a.txt")))).is_some());
    });
}
