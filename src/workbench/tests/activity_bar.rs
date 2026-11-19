//! Where the activity bar goes (`explorer`「活动栏与侧栏视图」), and
//! switching views without it.
use super::*;

/// Which of the activity bar's parts are drawn.
#[derive(Debug, PartialEq, Eq)]
struct Drawn {
    icons: bool,
    gear: bool,
    title: bool,
}

impl Harness {
    fn drawn(&self, cx: &mut TestAppContext) -> Drawn {
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            Drawn {
                icons: window.try_find("activity-explorer").is_some()
                    && window.try_find("activity-scm").is_some(),
                gear: window.try_find("activity-manage").is_some(),
                title: window.try_find("side-bar-title-SOURCE CONTROL").is_some(),
            }
        })
    }

    fn side_view(&self, cx: &mut TestAppContext) -> SideView {
        self.view
            .read_with(cx, |view, _| view.repository.as_ref().unwrap().side_view)
    }
}

#[gpui_kit::test]
async fn the_activity_bar_goes_where_the_setting_says(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    let cases = [
        ("default", true, true, true),
        ("top", true, false, false),
        ("bottom", true, false, true),
        ("hidden", false, false, true),
    ];
    for (location, icons, gear, title) in cases {
        harness.write_settings(
            &format!("{{ \"workbench.activityBar.location\": \"{location}\" }}"),
            cx,
        );
        assert_eq!(harness.drawn(cx), Drawn { icons, gear, title }, "{location}");
    }

    // On top, the icons still switch the view, and the badge comes along.
    harness.write_settings("{ \"workbench.activityBar.location\": \"top\" }", cx);
    assert_eq!(harness.label("activity-scm-badge", cx).as_deref(), Some("1"));
    harness.click("activity-explorer", cx);
    assert_eq!(harness.side_view(cx), SideView::Explorer);
}

#[gpui_kit::test]
async fn shortcuts_switch_views_without_the_activity_bar(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    harness.write_settings("{ \"workbench.activityBar.location\": \"hidden\" }", cx);
    assert_eq!(harness.side_view(cx), SideView::SourceControl);

    harness.press("cmd-shift-e", cx);
    cx.run_until_parked();
    assert_eq!(harness.side_view(cx), SideView::Explorer);
    let view = harness.view.clone();
    harness.update(cx, |window, cx| {
        assert!(view.read(cx).explorer_focus.is_focused(window), "the tree has the keyboard")
    });

    harness.press("ctrl-shift-g", cx);
    cx.run_until_parked();
    assert_eq!(harness.side_view(cx), SideView::SourceControl);
    let message = harness
        .view
        .read_with(cx, |view, _| view.repository.as_ref().unwrap().message.clone());
    harness.update(cx, |window, cx| {
        assert!(message.read(cx).focus_handle(cx).contains_focused(window, cx))
    });
}

#[gpui_kit::test]
async fn the_shortcuts_do_nothing_without_a_folder(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.press("cmd-shift-e", cx);
    harness.press("ctrl-shift-g", cx);
    cx.run_until_parked();
    harness.view.read_with(cx, |view, _| assert!(view.repository.is_none()));
}
