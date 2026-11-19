//! The settings page (`settings-editor`).
use gpui_kit::test::TestQueryExt as _;

use super::*;

impl Harness {
    fn open_settings(&self, cx: &mut TestAppContext) {
        self.press("cmd-,", cx);
        cx.run_until_parked();
    }

    pub(super) fn shows_element(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
    }

    pub(super) fn is_selected(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.find(id).selected() == Some(true)
        })
    }

    /// Clicks the open menu's item labeled `label`.
    fn click_menu_item(&self, label: &str, cx: &mut TestAppContext) {
        // Past the menu's opening, which runs on the simulated clock.
        cx.executor().advance_clock(Duration::from_secs(1));
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            let item = window.find_by_label(label);
            let id = item.path().last().cloned().expect("an id");
            window.click(id, cx);
        });
        cx.run_until_parked();
    }

    fn search_focused(&self, cx: &mut TestAppContext) -> bool {
        let view = self.view.clone();
        self.update(cx, |window, cx| {
            let editor = view.read(cx).editors.settings.as_ref().expect("open").editor.clone();
            let search = editor.read(cx).search.clone();
            search.read(cx).focus_handle(cx).contains_focused(window, cx)
        })
    }

    pub(super) fn count(&self, cx: &mut TestAppContext) -> Option<String> {
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find("settings-count")?.label().map(str::to_owned)
        })
    }

}

fn setting(id: &str) -> String {
    format!("setting-{id}")
}

#[gpui_kit::test]
async fn the_page_opens_once_with_or_without_a_folder(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.open_settings(cx);
    assert_eq!(harness.tabs(cx), [TabKey::Settings]);
    assert!(harness.search_focused(cx), "the keyboard is in the search box");
    assert!(!harness.shows_element("breadcrumbs", cx), "no path to show");
    harness.open_settings(cx);
    assert_eq!(harness.tabs(cx), [TabKey::Settings]);
    harness.press("cmd-w", cx);
    cx.run_until_parked();
    assert!(harness.shows_element("open-folder", cx), "the welcome page again");

    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    let view = harness.view.clone();
    let key = harness.file_tab("a.txt");
    harness.update(cx, |window, cx| {
        view.update(cx, |view, cx| view.open_tab(key, false, window, cx))
    });
    cx.run_until_parked();
    // Picked by clicking: the diff tab takes no keyboard, the window does.
    let first = harness.tabs(cx)[0].clone();
    harness.click(tab_id(&first), cx);
    cx.run_until_parked();
    harness.open_settings(cx);
    assert_eq!(
        harness.tabs(cx),
        [diff_tab(Changes, "a.txt"), TabKey::Settings, harness.file_tab("a.txt")],
        "right of the active tab"
    );
    assert_eq!(harness.active_tab(cx), Some(TabKey::Settings));
}

#[gpui_kit::test]
async fn the_gear_opens_the_settings(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.click("activity-manage", cx);
    cx.run_until_parked();
    harness.click_menu_item("设置", cx);
    assert_eq!(harness.active_tab(cx), Some(TabKey::Settings));
}

#[gpui_kit::test]
async fn the_table_of_contents_follows_the_pick_and_the_width(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.open_settings(cx);
    assert!(harness.is_selected("settings-toc-外观", cx));
    harness.click("settings-toc-Git", cx);
    assert!(harness.is_selected("settings-toc-Git", cx));
    assert!(!harness.is_selected("settings-toc-外观", cx));

    let window = VisualTestContext::from_window(harness.handle, cx);
    window.simulate_resize(size(px(650.), px(800.)));
    window.run_until_parked();
    assert!(!harness.shows_element("settings-toc", cx), "too narrow for it");
    assert!(harness.shows_element(setting("git.confirmSync"), cx));
}

#[gpui_kit::test]
async fn a_change_writes_the_file_and_marks_the_setting(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.open_settings(cx);
    let smart = setting("git.enableSmartCommit");
    assert!(!harness.shows_element(format!("{smart}-modified"), cx));

    harness.click(format!("{smart}-control"), cx);
    cx.run_until_parked();
    assert_eq!(
        harness.settings_text(),
        "{\n    \"git.enableSmartCommit\": true\n}\n"
    );
    assert!(harness.shows_element(format!("{smart}-modified"), cx));
    assert!(harness.is_selected(format!("{smart}-control"), cx), "checked");

    let location = setting("workbench.activityBar.location");
    harness.click(format!("{location}-control"), cx);
    cx.run_until_parked();
    harness.click_menu_item("top", cx);
    cx.read(|cx| {
        assert_eq!(
            SettingsStore::global(cx).values().activity_bar_location,
            crate::settings::ActivityBarLocation::Top
        )
    });
    assert!(harness.shows_element(format!("{location}-modified"), cx));

    harness.click(format!("{location}-control"), cx);
    cx.run_until_parked();
    harness.click_menu_item("default", cx);
    assert!(!harness.settings_text().contains("workbench.activityBar.location"));
    assert!(!harness.shows_element(format!("{location}-modified"), cx));
}

#[gpui_kit::test]
async fn the_gear_menu_resets_and_copies(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.write_settings("{ \"git.confirmSync\": false }", cx);
    harness.open_settings(cx);
    let sync = setting("git.confirmSync");

    harness.click(format!("{sync}-more"), cx);
    cx.run_until_parked();
    harness.click_menu_item("复制设置 id", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some("git.confirmSync")
    );

    harness.click(format!("{sync}-more"), cx);
    cx.run_until_parked();
    harness.click_menu_item("将设置复制为 JSON 文本", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some("\"git.confirmSync\": false")
    );

    harness.click(format!("{sync}-more"), cx);
    cx.run_until_parked();
    harness.click_menu_item("重置此设置", cx);
    assert!(!harness.settings_text().contains("git.confirmSync"));
    cx.read(|cx| assert!(SettingsStore::global(cx).values().confirm_sync));
}

#[gpui_kit::test]
async fn searching_narrows_the_list(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.write_settings("{ \"git.confirmSync\": false }", cx);
    harness.open_settings(cx);
    assert_eq!(harness.count(cx), None, "no count without a search");

    harness.type_text("smart", cx);
    assert_eq!(harness.count(cx).as_deref(), Some("找到 2 个设置"));
    assert!(harness.shows_element(setting("git.enableSmartCommit"), cx));
    assert!(harness.shows_element(setting("git.suggestSmartCommit"), cx));
    assert!(!harness.shows_element(setting("git.confirmSync"), cx));
    assert!(!harness.shows_element("settings-heading-外观", cx));

    harness.press("escape", cx);
    cx.run_until_parked();
    assert_eq!(harness.count(cx), None);
    assert!(harness.shows_element(setting("git.confirmSync"), cx));

    // The filter menu's 「已更改」 is `@modified`.
    harness.click("settings-filter", cx);
    cx.run_until_parked();
    harness.click_menu_item("已更改", cx);
    assert_eq!(harness.count(cx).as_deref(), Some("找到 1 个设置"));
    assert!(harness.shows_element(setting("git.confirmSync"), cx));

    harness.press("escape", cx);
    cx.run_until_parked();
    harness.type_text("同步", cx);
    assert_eq!(harness.count(cx).as_deref(), Some("找到 1 个设置"), "Chinese words too");

    harness.press("escape", cx);
    harness.type_text("font", cx);
    assert_eq!(harness.count(cx).as_deref(), Some("未找到设置"));
    assert!(!harness.shows_element("settings-toc", cx));
    assert!(!harness.shows_element("settings-list", cx));
}

#[gpui_kit::test]
async fn cmd_f_goes_back_to_the_search(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.open_settings(cx);
    let view = harness.view.clone();
    harness.update(cx, |window, cx| {
        let editor = view.read(cx).editors.settings.as_ref().unwrap().editor.clone();
        window.focus(&editor.read(cx).focus_handle(cx), cx);
    });
    assert!(!harness.search_focused(cx));
    harness.press("cmd-f", cx);
    cx.run_until_parked();
    assert!(harness.search_focused(cx));
}

#[gpui_kit::test]
async fn the_json_button_opens_and_if_need_be_makes_the_file(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.open_settings(cx);
    assert!(!harness.settings_file().exists());
    harness.click("settings-open-json", cx);
    cx.run_until_parked();
    assert_eq!(harness.settings_text(), "{\n}");
    cx.read(|cx| assert_eq!(*SettingsStore::global(cx).values(), Default::default()));
    let key = TabKey::File(crate::json_file::real_path(&harness.settings_file()).into());
    assert_eq!(harness.active_tab(cx), Some(key.clone()));
    assert_eq!(harness.tabs(cx), [TabKey::Settings, key]);
}
