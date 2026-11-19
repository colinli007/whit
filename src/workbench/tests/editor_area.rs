//! The editor area belongs to the window (`workbench-layout`「编辑区」,
//! `open-repository`): tabs of files outside the repository outlive it, and
//! with no folder open the welcome page is what the empty area shows.
use super::*;

/// A file outside any repository, by the path its tab is keyed on.
fn outside_file(folder: &TempDir, name: &str) -> TabKey {
    write(folder.path(), name, "notes\n");
    TabKey::File(canonical(folder.path()).join(name).into())
}

impl Harness {
    fn open_key(&self, key: TabKey, cx: &mut TestAppContext) {
        let view = self.view.clone();
        self.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.open_tab(key, false, window, cx))
        });
        cx.run_until_parked();
    }

    fn shows(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
    }
}

#[gpui_kit::test]
async fn outside_tabs_stay_when_another_folder_opens(cx: &mut TestAppContext) {
    let first = repository_with_changes(&["a.txt"]);
    let second = repository_with_changes(&["b.txt"]);
    let elsewhere = tempfile::tempdir().expect("temporary directory");
    let notes = outside_file(&elsewhere, "notes.txt");
    let harness = open_window(Some(first.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.open_key(harness.file_tab("a.txt"), cx);
    harness.open_key(notes.clone(), cx);
    // Opened for good, the file has the keyboard.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });
    cx.run_until_parked();

    harness.press("cmd-o", cx);
    let second_path = second.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![second_path]));
    cx.run_until_parked();
    // The edited file is not the repository's: nothing to ask.
    assert!(!cx.has_pending_prompt());
    harness.wait_for(&[(Changes, "b.txt")], cx).await;
    assert_eq!(harness.root(cx), Some(canonical(second.path())));
    let tabs = harness.tabs(cx);
    assert!(tabs.contains(&notes), "{tabs:?}");
    assert!(!tabs.contains(&harness.file_tab("a.txt")), "{tabs:?}");
    assert!(!tabs.contains(&diff_tab(Changes, "a.txt")), "{tabs:?}");
    let TabKey::File(path) = &notes else {
        unreachable!()
    };
    let dirty = harness.view.read_with(cx, |view, cx| {
        view.editors.buffers[path].buffer.read(cx).is_dirty(cx)
    });
    assert!(dirty, "the edit is kept");
}

#[gpui_kit::test]
async fn the_welcome_page_is_the_empty_editor_area(cx: &mut TestAppContext) {
    let elsewhere = tempfile::tempdir().expect("temporary directory");
    let notes = outside_file(&elsewhere, "notes.txt");
    let harness = open_window(None, cx);
    assert!(harness.shows("open-folder", cx));
    assert!(!harness.shows(tab_id(&notes), cx));
    assert!(!harness.shows("activity-explorer", cx));

    harness.open_key(notes.clone(), cx);
    assert!(harness.shows(tab_id(&notes), cx));
    assert!(!harness.shows("open-folder", cx));
    assert!(!harness.shows("activity-explorer", cx));

    harness.press("cmd-w", cx);
    cx.run_until_parked();
    assert!(harness.tabs(cx).is_empty());
    assert!(harness.shows("open-folder", cx));
    assert!(!harness.shows(tab_id(&notes), cx));
}

#[gpui_kit::test]
async fn closing_the_folder_keeps_the_other_tabs(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let elsewhere = tempfile::tempdir().expect("temporary directory");
    let notes = outside_file(&elsewhere, "notes.txt");
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.open_key(harness.file_tab("a.txt"), cx);
    harness.open_key(notes.clone(), cx);

    harness.update(cx, |window, cx| window.dispatch_action(Box::new(CloseFolder), cx));
    cx.run_until_parked();
    harness.view.read_with(cx, |view, _| assert!(view.repository.is_none()));
    assert_eq!(harness.tabs(cx), std::slice::from_ref(&notes));
    assert!(harness.shows(tab_id(&notes), cx));
    assert!(!harness.shows("activity-explorer", cx));
    assert!(!harness.shows("open-folder", cx));

    harness.press("cmd-w", cx);
    cx.run_until_parked();
    assert!(harness.shows("open-folder", cx));
}
