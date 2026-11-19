//! Asking about unsaved edits before they are dropped (`file-editor`
//! 「关闭前询问保存」, `open-repository`).
use std::fs;

use super::*;

/// A repository with `a.txt` and `b.txt` changed, both open in file tabs and
/// edited; `a.txt` is the active tab.
async fn edited(cx: &mut TestAppContext) -> (TempDir, Harness) {
    let repository = repository_with_changes(&["a.txt", "b.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx).await;
    for name in ["b.txt", "a.txt"] {
        let view = harness.view.clone();
        harness.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.open_tab(harness.file_tab(name), false, window, cx))
        });
        cx.run_until_parked();
        // Opened for good, the file has the keyboard.
        harness.update(cx, |window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });
        cx.run_until_parked();
    }
    (repository, harness)
}

fn read(repository: &TempDir, name: &str) -> String {
    fs::read_to_string(repository.path().join(name)).expect("read file")
}

#[gpui_kit::test]
async fn closing_an_edited_tab_asks_first(cx: &mut TestAppContext) {
    let (repository, harness) = edited(cx).await;

    harness.press("cmd-w", cx);
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    cx.run_until_parked();
    assert!(harness.tabs(cx).contains(&harness.file_tab("a.txt")));

    harness.press("cmd-w", cx);
    cx.run_until_parked();
    cx.simulate_prompt_answer("不保存");
    cx.run_until_parked();
    assert!(!harness.tabs(cx).contains(&harness.file_tab("a.txt")));
    assert_eq!(read(&repository, "a.txt"), "after\n");

    // `b.txt` is active now.
    harness.press("cmd-w", cx);
    cx.run_until_parked();
    cx.simulate_prompt_answer("保存");
    cx.run_until_parked();
    assert!(!harness.tabs(cx).contains(&harness.file_tab("b.txt")));
    assert_eq!(read(&repository, "b.txt"), "xafter\n");
}

#[gpui_kit::test]
async fn closing_the_folder_asks_about_every_edited_file(cx: &mut TestAppContext) {
    let (repository, harness) = edited(cx).await;

    harness.update(cx, |window, cx| window.dispatch_action(Box::new(CloseFolder), cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("全部保存");
    cx.run_until_parked();
    assert_eq!(read(&repository, "a.txt"), "xafter\n");
    assert_eq!(read(&repository, "b.txt"), "xafter\n");
    harness.view.read_with(cx, |view, _| assert!(view.repository.is_none()));
}

#[gpui_kit::test]
async fn canceling_keeps_the_repository_open(cx: &mut TestAppContext) {
    let (repository, harness) = edited(cx).await;
    let other = repository_with_changes(&["other.txt"]);

    harness.press("cmd-o", cx);
    let other_path = other.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![other_path]));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    cx.run_until_parked();
    assert_eq!(harness.root(cx), Some(canonical(repository.path())));
    assert!(harness.tabs(cx).contains(&harness.file_tab("a.txt")));

    // Not saving goes on to the other repository, leaving the files alone.
    harness.update(cx, |window, cx| window.dispatch_action(Box::new(CloseFolder), cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("不保存");
    cx.run_until_parked();
    assert_eq!(read(&repository, "a.txt"), "after\n");
    harness.view.read_with(cx, |view, _| assert!(view.repository.is_none()));
}

#[gpui_kit::test]
async fn quitting_saves_every_edited_file_when_asked(cx: &mut TestAppContext) {
    let (repository, harness) = edited(cx).await;

    harness.press("cmd-q", cx);
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("全部保存");
    cx.run_until_parked();
    assert_eq!(read(&repository, "a.txt"), "xafter\n");
    assert_eq!(read(&repository, "b.txt"), "xafter\n");
}

#[gpui_kit::test]
async fn nothing_to_ask_without_unsaved_edits(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    let view = harness.view.clone();
    harness.update(cx, |window, cx| {
        view.update(cx, |view, cx| view.open_tab(harness.file_tab("a.txt"), false, window, cx))
    });
    cx.run_until_parked();

    harness.press("cmd-w", cx);
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(!harness.tabs(cx).contains(&harness.file_tab("a.txt")));
}

#[gpui_kit::test]
async fn the_close_button_asks_first(cx: &mut TestAppContext) {
    let (repository, harness) = edited(cx).await;

    let mut window = VisualTestContext::from_window(harness.handle, cx);
    assert!(!window.simulate_close(), "kept open to ask");
    window.run_until_parked();
    assert!(window.has_pending_prompt());
    window.simulate_prompt_answer("取消");
    window.run_until_parked();
    assert!(cx.update_window(harness.handle, |_, _, _| ()).is_ok());

    let mut window = VisualTestContext::from_window(harness.handle, cx);
    assert!(!window.simulate_close());
    window.run_until_parked();
    window.simulate_prompt_answer("全部保存");
    window.run_until_parked();
    assert_eq!(read(&repository, "a.txt"), "xafter\n");
    assert!(cx.update_window(harness.handle, |_, _, _| ()).is_err(), "the window closed");
}
