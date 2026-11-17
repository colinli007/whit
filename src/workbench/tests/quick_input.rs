//! The picker and input box, driven through the window as a user would.
use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::workbench::quick_input::{
    self, InputOptions, PickGroup, PickItem, PickList, QuickInput, QuickPick, Validation,
};
use core::prelude::v1::test;

/// What a flow received: unset while it still waits, then its answer.
type Answer<T> = Rc<RefCell<Option<Option<T>>>>;

fn sample_list() -> PickList<&'static str> {
    let item = |label: &'static str| (PickItem::new(label), label);
    PickList {
        actions: vec![item("创建新分支..."), item("创建新分支依据...")],
        groups: vec![
            PickGroup {
                label: Some("分支".into()),
                items: vec![item("main"), item("main-next"), item("dev")],
            },
            PickGroup {
                label: Some("远程分支".into()),
                items: vec![item("origin/main")],
            },
        ],
    }
}

impl Harness {
    /// Starts a flow that awaits a picker filled from `list`.
    fn start_pick(
        &self,
        list: impl Future<Output = Result<PickList<&'static str>, GitError>> + 'static,
        cx: &mut TestAppContext,
    ) -> Answer<(&'static str, String)> {
        let answer: Answer<(&'static str, String)> = Rc::default();
        let view = self.view.clone();
        let sink = answer.clone();
        self.update(cx, |window, cx| {
            view.update(cx, |_, cx| {
                cx.spawn_in(window, async move |this, cx| {
                    let picked = quick_input::pick(&this, cx, "选择要签出的分支或标记", list).await;
                    *sink.borrow_mut() = Some(picked.map(|picked| (picked.value, picked.query)));
                })
                .detach()
            })
        });
        cx.run_until_parked();
        answer
    }

    fn start_input(&self, value: &str, cx: &mut TestAppContext) -> Answer<String> {
        let answer: Answer<String> = Rc::default();
        let view = self.view.clone();
        let sink = answer.clone();
        let options = InputOptions {
            placeholder: "分支名称".into(),
            prompt: "请提供新的分支名称".into(),
            value: value.to_owned(),
            validate: Box::new(|value| {
                let name = crate::git::sanitize_branch_name(value);
                if name == "dev" {
                    Validation::Error(format!("分支“{name}”已存在").into())
                } else if name != value {
                    Validation::Info(format!("新分支将为“{name}”").into())
                } else {
                    Validation::Valid
                }
            }),
        };
        self.update(cx, |window, cx| {
            view.update(cx, |_, cx| {
                cx.spawn_in(window, async move |this, cx| {
                    let typed = quick_input::input(&this, cx, options).await;
                    *sink.borrow_mut() = Some(typed);
                })
                .detach()
            })
        });
        cx.run_until_parked();
        answer
    }

    pub(super) fn quick_pick(&self, cx: &mut TestAppContext) -> Option<Entity<QuickPick>> {
        self.view.read_with(cx, |view, _| match &view.quick_input {
            Some((QuickInput::Pick(pick), _)) => Some(pick.clone()),
            _ => None,
        })
    }

    pub(super) fn has_quick_input(&self, cx: &mut TestAppContext) -> bool {
        self.view
            .read_with(cx, |view, _| view.quick_input.is_some())
    }

    pub(super) fn shown(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.update(cx, |window, cx| window.render_frame(cx));
        let pick = self.quick_pick(cx).expect("a picker");
        pick.read_with(cx, |pick, _| pick.shown_labels())
    }

    pub(super) fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.input(text, cx);
        });
        cx.run_until_parked();
    }

    pub(super) fn key(&self, key: &str, cx: &mut TestAppContext) {
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        });
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
async fn the_picker_filters_and_orders_like_vs_code(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    let answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    assert_eq!(
        harness.shown(cx),
        [
            "创建新分支...",
            "创建新分支依据...",
            "—",
            "main [分支]",
            "main-next",
            "dev",
            "—",
            "origin/main [远程分支]",
        ]
    );

    // Typing reaches the query field: it has focus as soon as it opens.
    harness.type_text("MAIN", cx);
    assert_eq!(
        harness.shown(cx),
        [
            "main [分支]",
            "main-next",
            "—",
            "origin/main [远程分支]",
            "—",
            "创建新分支...",
            "创建新分支依据...",
        ]
    );

    // The highlight starts on the first match; Down then Enter confirms the next.
    harness.key("down", cx);
    harness.key("enter", cx);
    assert_eq!(
        *answer.borrow(),
        Some(Some(("main-next", "MAIN".to_owned())))
    );
    assert!(!harness.has_quick_input(cx));
}

#[gpui_kit::test]
async fn only_the_actions_remain_when_nothing_matches(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    let _answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    harness.type_text("xyz", cx);
    assert_eq!(harness.shown(cx), ["创建新分支...", "创建新分支依据..."]);
}

#[gpui_kit::test]
async fn escape_clicking_outside_and_leaving_the_window_dismiss(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    // Escape closes at once, even with text typed (VS Code does not first
    // clear the query).
    let answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    harness.type_text("ma", cx);
    harness.key("escape", cx);
    assert_eq!(*answer.borrow(), Some(None));
    assert!(!harness.has_quick_input(cx));

    let answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    harness.click("status-branch", cx);
    cx.run_until_parked();
    assert_eq!(*answer.borrow(), Some(None));

    harness.update(cx, |window, _| window.activate_window());
    cx.run_until_parked();
    let answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    VisualTestContext::from_window(harness.handle, cx).deactivate_window();
    cx.run_until_parked();
    assert_eq!(*answer.borrow(), Some(None));
    assert!(!harness.has_quick_input(cx));
}

#[gpui_kit::test]
async fn opening_another_quick_input_answers_the_first_with_nothing(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    let first = harness.start_pick(async { Ok(sample_list()) }, cx);
    let second = harness.start_pick(async { Ok(sample_list()) }, cx);
    assert_eq!(*first.borrow(), Some(None));
    assert_eq!(*second.borrow(), None);
    harness.key("enter", cx);
    assert_eq!(
        *second.borrow(),
        Some(Some(("创建新分支...", String::new())))
    );
}

#[gpui_kit::test]
async fn text_typed_while_loading_filters_the_items_once_they_arrive(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    let (send, receive) = futures_channel::oneshot::channel();
    let answer = harness.start_pick(
        async move { Ok(receive.await.expect("the list is sent")) },
        cx,
    );
    let pick = harness.quick_pick(cx).expect("the picker opens at once");
    assert!(pick.read_with(cx, |pick, cx| pick.is_loading(cx)));
    harness.type_text("dev", cx);

    let _ = send.send(sample_list());
    cx.run_until_parked();
    assert!(!pick.read_with(cx, |pick, cx| pick.is_loading(cx)));
    assert_eq!(
        harness.shown(cx),
        ["dev [分支]", "—", "创建新分支...", "创建新分支依据..."]
    );
    harness.key("enter", cx);
    assert_eq!(*answer.borrow(), Some(Some(("dev", "dev".to_owned()))));
}

#[gpui_kit::test]
async fn jumping_to_the_ends_and_by_pages(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    let answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    harness.key("cmd-down", cx);
    harness.key("cmd-up", cx);
    harness.key("cmd-down", cx);
    harness.key("enter", cx);
    assert_eq!(*answer.borrow(), Some(Some(("origin/main", String::new()))));

    let answer = harness.start_pick(async { Ok(sample_list()) }, cx);
    harness.key("pagedown", cx);
    harness.key("pageup", cx);
    harness.key("enter", cx);
    assert_eq!(
        *answer.borrow(),
        Some(Some(("创建新分支...", String::new())))
    );
}

#[gpui_kit::test]
async fn the_input_box_validates_as_it_is_typed(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    let validation = |harness: &Harness, cx: &mut TestAppContext| {
        harness
            .view
            .read_with(cx, |view, cx| match &view.quick_input {
                Some((QuickInput::Input(input), _)) => input.read(cx).validation().clone(),
                _ => panic!("an input box"),
            })
    };

    // The initial value is selected, so typing replaces it.
    let answer = harness.start_input("old", cx);
    harness.type_text("dev", cx);
    assert_eq!(
        validation(&harness, cx),
        Validation::Error("分支“dev”已存在".into())
    );
    harness.key("enter", cx);
    assert_eq!(*answer.borrow(), None, "an error refuses Enter");

    // Clicking elsewhere leaves it open.
    harness.click("scm-refresh", cx);
    cx.run_until_parked();
    assert!(harness.has_quick_input(cx));

    harness.update(cx, |window, cx| {
        let Some((QuickInput::Input(input), _)) = &harness.view.read(cx).quick_input else {
            panic!("an input box");
        };
        let input = input.clone();
        input.update(cx, |input, cx| input.focus_for_test(window, cx));
    });
    harness.key("cmd-a", cx);
    harness.type_text("fix login", cx);
    assert_eq!(
        validation(&harness, cx),
        Validation::Info("新分支将为“fix-login”".into())
    );
    harness.key("enter", cx);
    assert_eq!(*answer.borrow(), Some(Some("fix login".to_owned())));
    assert!(!harness.has_quick_input(cx));

    let answer = harness.start_input("", cx);
    harness.key("escape", cx);
    assert_eq!(*answer.borrow(), Some(None));
}
