//! Writing the user settings from the window (`settings`「拒绝写入」,
//! 「设置文件有语法错误」). The watcher is off in unit tests: they hand the
//! file's new text to the store as the watcher would.
use std::fs;

use super::*;

impl Harness {
    pub(super) fn settings_file(&self) -> PathBuf {
        self._recent.path().join(crate::settings::FILE_NAME)
    }

    /// Writes the settings file and lets the store take it in, as the
    /// watcher does after a change made elsewhere.
    pub(super) fn write_settings(&self, text: &str, cx: &mut TestAppContext) {
        fs::write(self.settings_file(), text).expect("write settings");
        let text = text.to_owned();
        cx.update(|cx| {
            cx.update_global(|store: &mut SettingsStore, _| store.reload(Some(text)));
        });
        cx.run_until_parked();
    }

    pub(super) fn settings_text(&self) -> String {
        fs::read_to_string(self.settings_file()).expect("read settings")
    }

    pub(super) fn write_setting(&self, id: &'static str, value: bool, cx: &mut TestAppContext) -> bool {
        let view = self.view.clone();
        let written = self.update(cx, |window, cx| {
            view.update(cx, |view, cx| view.write_setting(id, value.into(), window, cx))
        });
        cx.run_until_parked();
        written
    }
}

#[gpui_kit::test]
async fn unsaved_edits_to_the_file_refuse_a_write(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.write_settings("{}\n", cx);
    harness.update(cx, |window, cx| {
        window.dispatch_action(Box::new(crate::OpenUserSettingsJson), cx)
    });
    cx.run_until_parked();
    // Opened for good, the file has the keyboard.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.input(" ", cx);
    });
    cx.run_until_parked();

    assert!(!harness.write_setting("git.confirmSync", false, cx));
    assert_eq!(harness.settings_text(), "{}\n");
    cx.read(|cx| assert!(SettingsStore::global(cx).values().confirm_sync));
    assert_eq!(harness.notification_count(cx), 1);

    // Past the notification's slide-in, which runs on the simulated clock.
    cx.executor().advance_clock(Duration::from_secs(1));
    harness.click("settings-save-and-retry", cx);
    cx.run_until_parked();
    assert_eq!(harness.settings_text(), " {\n    \"git.confirmSync\": false\n}\n");
    cx.read(|cx| assert!(!SettingsStore::global(cx).values().confirm_sync));
}

#[gpui_kit::test]
async fn a_broken_file_is_told_about_and_not_written(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.write_settings("{ \"workbench.activityBar.location\": \"top\" }", cx);
    harness.write_settings("{ \"workbench.activityBar.location\": ", cx);
    assert_eq!(harness.notification_count(cx), 1, "the file does not parse");
    cx.read(|cx| {
        let store = SettingsStore::global(cx);
        assert_eq!(
            store.values().activity_bar_location,
            crate::settings::ActivityBarLocation::Top,
            "the last settings stay"
        );
    });

    assert!(!harness.write_setting("git.confirmSync", false, cx));
    assert_eq!(harness.settings_text(), "{ \"workbench.activityBar.location\": ");
    assert_eq!(harness.notification_count(cx), 2);

    // Fixed, the file's notice goes away and its settings apply.
    harness.write_settings("{ \"workbench.activityBar.location\": \"bottom\" }", cx);
    // Past the closing notice's slide-out.
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(harness.notification_count(cx), 1, "only the refused write's notice is left");
    cx.read(|cx| {
        assert_eq!(
            SettingsStore::global(cx).values().activity_bar_location,
            crate::settings::ActivityBarLocation::Bottom
        );
    });
}
