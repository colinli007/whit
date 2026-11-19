//! Files open from outside the repository follow the disk (`file-editor`
//! 「多标签」), and `settings.json` edited in a tab takes effect
//! (`settings`「外部修改立即生效」). Unit tests hand in the watchers' batches.
use std::{collections::BTreeSet, fs, sync::Arc};

use super::*;
use crate::watcher::FileBatch;

impl Harness {
    fn open_settings_json(&self, cx: &mut TestAppContext) -> Arc<Path> {
        self.update(cx, |window, cx| {
            window.dispatch_action(Box::new(crate::OpenUserSettingsJson), cx)
        });
        cx.run_until_parked();
        crate::json_file::real_path(&self.settings_file()).into()
    }

    /// What the folder watcher would report after `path` changed.
    fn outside_changed(&self, path: &Path, cx: &mut TestAppContext) {
        let batch = FileBatch {
            paths: BTreeSet::from([path.to_owned()]),
            rescan: false,
        };
        let view = self.view.clone();
        self.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.outside_files_changed(&batch, window, cx))
        });
        cx.run_until_parked();
    }

    fn buffer_text(&self, path: &Path, cx: &mut TestAppContext) -> String {
        self.view.read_with(cx, |view, cx| {
            let buffer = view.editors.buffers[path].buffer.read(cx);
            buffer.editor().expect("text").read(cx).value().to_string()
        })
    }

    fn watched_folders(&self, cx: &mut TestAppContext) -> Vec<PathBuf> {
        self.view.read_with(cx, |view, _| {
            view.outside_watcher
                .as_ref()
                .map(|watching| watching.folders().iter().cloned().collect())
                .unwrap_or_default()
        })
    }
}

#[gpui_kit::test]
async fn an_outside_file_follows_the_disk(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.write_settings("{}\n", cx);
    let path = harness.open_settings_json(cx);
    assert_eq!(harness.watched_folders(cx), [path.parent().unwrap().to_owned()]);

    // Saved, it reloads quietly.
    fs::write(&path, "{ \"git.confirmSync\": false }\n").unwrap();
    harness.outside_changed(&path, cx);
    assert_eq!(harness.buffer_text(&path, cx), "{ \"git.confirmSync\": false }\n");

    // With unsaved edits it is kept, and saving asks to overwrite.
    harness.type_text(" ", cx);
    fs::write(&path, "{ \"theirs\": 1 }\n").unwrap();
    harness.outside_changed(&path, cx);
    assert!(harness.buffer_text(&path, cx).starts_with(' '));
    harness.press("cmd-s", cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    assert!(harness.shows_element("save-overwrite", cx), "the conflict offers to overwrite");

    // Deleted, its tab is struck through.
    fs::remove_file(&path).unwrap();
    harness.outside_changed(&path, cx);
    let deleted = harness
        .view
        .read_with(cx, |view, cx| view.editors.buffers[&path].buffer.read(cx).is_deleted());
    assert!(deleted);

    // Closed, its folder is no longer watched.
    harness.press("cmd-w", cx);
    cx.run_until_parked();
    cx.simulate_prompt_answer("不保存");
    cx.run_until_parked();
    assert!(harness.watched_folders(cx).is_empty());
}

#[gpui_kit::test]
async fn saving_settings_json_applies_it(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.write_settings("{}", cx);
    harness.press("cmd-,", cx);
    cx.run_until_parked();
    let path = harness.open_settings_json(cx);
    // Opened for good, the file has the keyboard: replace all it holds.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.press("cmd-a", cx);
        window.input("{ \"workbench.activityBar.location\": \"top\" }", cx);
    });
    cx.run_until_parked();
    harness.press("cmd-s", cx);
    cx.run_until_parked();
    // The settings watcher takes the saved text in.
    let saved = fs::read_to_string(&path).unwrap();
    harness.write_settings(&saved, cx);

    assert!(!harness.shows_element("activity-manage", cx), "the activity bar is on top");
    harness.click(tab_id(&TabKey::Settings), cx);
    cx.run_until_parked();
    let location = "setting-workbench.activityBar.location";
    assert!(harness.shows_element(format!("{location}-modified"), cx));
}

#[gpui_kit::test]
async fn the_page_follows_a_dialog_s_write_and_keeps_the_search(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.press("cmd-,", cx);
    cx.run_until_parked();
    harness.type_text("git", cx);
    assert_eq!(harness.count(cx).as_deref(), Some("找到 3 个设置"));

    // As 「确定，且不再显示」 does.
    assert!(harness.write_setting("git.confirmSync", false, cx));
    assert!(harness.shows_element("setting-git.confirmSync-modified", cx));
    assert!(!harness.is_selected("setting-git.confirmSync-control", cx), "unchecked");
    assert_eq!(harness.count(cx).as_deref(), Some("找到 3 个设置"), "the search stays");
}
