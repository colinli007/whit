use super::*;

mod branches;
mod quick_input;

// `super::*` carries GPUI's `test` macro in through the `gpui_kit::*` glob;
// `#[gpui_kit::test]` expands to `#[test]`, which must stay the built-in.
use crate::git::fixtures::{add_remote, commit_all, git, git_output, new_repository, write};
use core::prelude::v1::test;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::diff::{DiffLineRange, DiffSide};
use gpui_kit::test::{TestAppContextExt as _, TestQueryExt as _, TestWindowExt as _};
use scm_panel::file_row_id;
use std::time::Duration;
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

struct Harness {
    handle: AnyWindowHandle,
    view: Entity<Workbench>,
    /// Holds the recent-folder list, kept out of the user's own.
    _recent: TempDir,
}

fn open_window(folder: Option<PathBuf>, cx: &mut TestAppContext) -> Harness {
    let recent = tempfile::tempdir().expect("temporary directory");
    open_window_with_recent(folder, recent, cx)
}

fn open_window_with_recent(
    folder: Option<PathBuf>,
    recent: TempDir,
    cx: &mut TestAppContext,
) -> Harness {
    let recent_file = recent.path().join("recent.json");
    // The settings of a test are not the user's either.
    let settings_file = recent.path().join("settings.json");
    let (handle, view) = cx.update(|cx| {
        crate::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1200.), px(800.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                Workbench::new(
                    folder,
                    RecentFolders::load_from(Some(recent_file)),
                    crate::Settings::load_from(Some(settings_file)),
                    window,
                    cx,
                )
            })
        })
        .expect("open window")
    });
    Harness {
        handle,
        view,
        _recent: recent,
    }
}

impl Harness {
    fn update<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        cx.update_window(self.handle, |_, window, cx| f(window, cx))
            .expect("window")
    }

    fn click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        });
    }

    /// Hovers a row so its action buttons show, then clicks one.
    fn click_row_action(
        &self,
        row: SharedString,
        action: impl Into<ElementId>,
        cx: &mut TestAppContext,
    ) {
        let action = action.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.hover(row, cx);
            window.render_frame(cx);
            window.click(action, cx);
        });
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.update(cx, |window, cx| window.press(key, cx));
    }

    /// Group and path of every changed file once a load has finished.
    fn loaded(&self, cx: &App) -> Option<Vec<(ChangeGroup, String)>> {
        let repository = self.view.read(cx).repository.as_ref()?;
        matches!(repository.load_state, LoadState::Loaded).then(|| {
            repository
                .files()
                .iter()
                .map(|changed| (changed.group, changed.file.path().to_string()))
                .collect()
        })
    }

    async fn wait_for(&self, expected: &[(ChangeGroup, &str)], cx: &mut TestAppContext) {
        let expected: Vec<(ChangeGroup, String)> = expected
            .iter()
            .map(|(group, path)| (*group, path.to_string()))
            .collect();
        let view = self.view.clone();
        let probe = Harness::probe(view);
        cx.wait_for(self.handle, TIMEOUT, move |_, cx| {
            probe(cx).is_some_and(|loaded| loaded == expected)
        })
        .await;
    }

    fn probe(view: Entity<Workbench>) -> impl Fn(&App) -> Option<Vec<(ChangeGroup, String)>> {
        move |cx| {
            let repository = view.read(cx).repository.as_ref()?;
            matches!(repository.load_state, LoadState::Loaded).then(|| {
                repository
                    .files()
                    .iter()
                    .map(|changed| (changed.group, changed.file.path().to_string()))
                    .collect()
            })
        }
    }

    /// Group and path of the file in the editor, and what the diff holds.
    fn open(&self, cx: &mut TestAppContext) -> (Option<(ChangeGroup, String)>, Vec<String>) {
        self.view.read_with(cx, |view, cx| {
            let repository = view.repository.as_ref().expect("repository");
            let open = repository
                .open
                .as_ref()
                .map(|open| (open.group, open.path.to_string()));
            let diff = repository
                .diff
                .read(cx)
                .files()
                .iter()
                .map(|file| file.path().to_string())
                .collect();
            (open, diff)
        })
    }

    fn label(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> Option<String> {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.find(id).label().map(str::to_owned)
        })
    }

    fn is_disabled(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.find(id).disabled() == Some(true)
        })
    }

    fn root(&self, cx: &mut TestAppContext) -> Option<PathBuf> {
        self.view.read_with(cx, |view, _| {
            view.repository
                .as_ref()
                .map(|repository| canonical(&repository.root))
        })
    }

    fn recent(&self, cx: &mut TestAppContext) -> Vec<PathBuf> {
        self.view
            .read_with(cx, |view, _| view.recent.folders().to_vec())
    }

    fn type_message(&self, text: &str, cx: &mut TestAppContext) {
        let view = self.view.clone();
        self.update(cx, |window, cx| {
            let message = view
                .read(cx)
                .repository
                .as_ref()
                .expect("repository")
                .message
                .clone();
            message.update(cx, |state, cx| state.focus(window, cx));
            window.render_frame(cx);
            window.input(text, cx);
        });
    }

    fn notification_count(&self, cx: &mut TestAppContext) -> usize {
        self.update(cx, |window, cx| window.notifications(cx).len())
    }

    fn message(&self, cx: &mut TestAppContext) -> String {
        self.view.read_with(cx, |view, cx| {
            view.repository
                .as_ref()
                .expect("repository")
                .message
                .read(cx)
                .value()
                .to_string()
        })
    }

    async fn wait_until_idle(&self, cx: &mut TestAppContext) {
        let view = self.view.clone();
        cx.wait_for(self.handle, TIMEOUT, move |_, cx| {
            view.read(cx)
                .repository
                .as_ref()
                .is_some_and(|repository| !repository.is_busy())
        })
        .await;
        cx.run_until_parked();
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().expect("canonical path")
}

/// A repository with each file committed as "before", then changed to "after".
fn repository_with_changes(names: &[&str]) -> TempDir {
    let repository = new_repository();
    let path = repository.path();
    for name in names {
        write(path, name, "before\n");
    }
    commit_all(path);
    for name in names {
        write(path, name, "after\n");
    }
    repository
}

use ChangeGroup::{Changes, Staged};

#[gpui_kit::test]
async fn opening_a_subfolder_through_the_dialog_shows_the_whole_repository(
    cx: &mut TestAppContext,
) {
    let repository = repository_with_changes(&["a.txt", "src/lib.rs"]);
    let harness = open_window(None, cx);

    harness.click("open-folder", cx);
    assert!(cx.did_prompt_for_paths());
    let subfolder = repository.path().join("src");
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![subfolder])
    });

    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "src/lib.rs")], cx)
        .await;
    assert_eq!(harness.root(cx), Some(canonical(repository.path())));
}

#[gpui_kit::test]
async fn cancelling_the_dialog_keeps_the_open_repository(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    harness.press("cmd-o", cx);
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();

    assert_eq!(harness.root(cx), Some(canonical(repository.path())));
}

#[gpui_kit::test]
async fn opening_another_folder_replaces_the_repository(cx: &mut TestAppContext) {
    let first = repository_with_changes(&["first.txt"]);
    let second = repository_with_changes(&["second.txt"]);
    let harness = open_window(Some(first.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "first.txt")], cx).await;

    harness.press("cmd-o", cx);
    let second_path = second.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![second_path]));

    harness.wait_for(&[(Changes, "second.txt")], cx).await;
    assert_eq!(harness.root(cx), Some(canonical(second.path())));
}

#[gpui_kit::test]
async fn folder_outside_a_repository_explains_why_on_the_welcome_page(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().expect("temporary directory");
    let harness = open_window(Some(folder.path().to_path_buf()), cx);

    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window.try_find("open-error").is_some()
    })
    .await;
    assert_eq!(
        harness.label("open-error", cx).as_deref(),
        Some("该文件夹不是 git 仓库")
    );
    harness.update(cx, |window, _| {
        assert!(window.try_find("open-folder").is_some());
    });
}

#[gpui_kit::test]
async fn clean_repository_says_there_is_nothing_to_show(cx: &mut TestAppContext) {
    let repository = new_repository();
    write(repository.path(), "a.txt", "same\n");
    commit_all(repository.path());
    let harness = open_window(Some(repository.path().to_path_buf()), cx);

    harness.wait_for(&[], cx).await;
    assert_eq!(
        harness.label("diff-message", cx).as_deref(),
        Some("没有未提交的改动")
    );
    harness.update(cx, |window, _| {
        assert!(window.try_find("scm-group-changes").is_none());
        assert!(window.try_find("scm-group-staged").is_none());
    });
}

#[gpui_kit::test]
async fn a_partly_staged_file_opens_from_either_group(cx: &mut TestAppContext) {
    let repository = new_repository();
    let path = repository.path();
    let lines: String = (1..=10).map(|line| format!("{line}\n")).collect();
    write(path, "a.txt", &lines);
    write(path, "b.txt", "b\n");
    commit_all(path);
    let staged = lines.replace("1\n", "one\n");
    write(path, "a.txt", &staged);
    git(path, &["add", "a.txt"]);
    write(path, "a.txt", staged.replace("10\n", "ten\n"));
    write(path, "b.txt", "b2\n");
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness
        .wait_for(
            &[(Staged, "a.txt"), (Changes, "a.txt"), (Changes, "b.txt")],
            cx,
        )
        .await;

    // The first file opens by itself.
    assert_eq!(
        harness.open(cx),
        (Some((Staged, "a.txt".into())), vec!["a.txt".into()])
    );
    assert_eq!(
        harness.label("editor-tab", cx).as_deref(),
        Some("a.txt (索引)")
    );

    harness.click(file_row_id(Changes, "a.txt"), cx);
    assert_eq!(harness.open(cx).0, Some((Changes, "a.txt".into())));
    assert_eq!(
        harness.label("editor-tab", cx).as_deref(),
        Some("a.txt (工作树)")
    );
    let patch = harness.view.read_with(cx, |view, _| {
        view.repository
            .as_ref()
            .unwrap()
            .open_file()
            .unwrap()
            .patch
            .clone()
    });
    assert!(patch.contains("+ten") && !patch.contains("+one"));

    harness.click(file_row_id(Changes, "b.txt"), cx);
    assert_eq!(
        harness.open(cx),
        (Some((Changes, "b.txt".into())), vec!["b.txt".into()])
    );
}

#[gpui_kit::test]
async fn closing_the_tab_leaves_the_editor_empty(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    harness.click("close-editor", cx);
    assert_eq!(harness.open(cx), (None, vec![]));
    assert_eq!(
        harness.label("diff-message", cx).as_deref(),
        Some("在左侧选择一个文件查看改动")
    );
}

#[gpui_kit::test]
async fn toggling_inline_keeps_the_file(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    harness.click("toggle-inline", cx);
    let mode = harness.view.read_with(cx, |view, cx| {
        view.repository.as_ref().unwrap().diff.read(cx).mode()
    });
    assert_eq!(mode, DiffMode::Unified);
    assert_eq!(harness.open(cx).1, ["a.txt"]);
}

#[gpui_kit::test]
async fn refresh_shortcut_reloads_the_changes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    write(repository.path(), "b.txt", "new\n");
    harness.press("cmd-r", cx);

    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx)
        .await;
}

#[gpui_kit::test]
async fn returning_to_the_window_reloads_the_changes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    let activate = |cx: &mut TestAppContext| {
        harness.update(cx, |window, _| window.activate_window());
        cx.run_until_parked();
    };

    // Test windows open inactive; the app's window becomes active on launch,
    // and that first activation is not a return from elsewhere.
    write(repository.path(), "b.txt", "new\n");
    activate(cx);
    cx.update(|cx| assert_eq!(harness.loaded(cx).map(|files| files.len()), Some(1)));

    VisualTestContext::from_window(harness.handle, cx).deactivate_window();
    activate(cx);

    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx)
        .await;
}

#[gpui_kit::test]
async fn changes_to_other_files_leave_the_shown_diff_alone(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    // Replacing the shown file would clear this, like its scroll position.
    let range = DiffLineRange::new("a.txt", DiffSide::Modified, 1, 1);
    let diff = harness
        .view
        .read_with(cx, |view, _| view.repository.as_ref().unwrap().diff.clone());
    diff.update(cx, |diff, cx| {
        diff.set_selected_lines(Some(range.clone()), cx)
    });

    write(repository.path(), "b.txt", "new\n");
    harness.press("cmd-r", cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx)
        .await;
    diff.read_with(cx, |diff, _| {
        assert_eq!(diff.selected_lines(), Some(range.clone()))
    });

    write(repository.path(), "a.txt", "edited\n");
    harness.press("cmd-r", cx);
    cx.wait_for(harness.handle, TIMEOUT, |_, cx| {
        diff.read(cx).selected_lines().is_none()
    })
    .await;
}

#[gpui_kit::test]
async fn staging_unstaging_and_discarding_from_the_rows(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt"]);
    let path = repository.path();
    write(path, "new.txt", "n\n");
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness
        .wait_for(
            &[(Changes, "a.txt"), (Changes, "b.txt"), (Changes, "new.txt")],
            cx,
        )
        .await;

    harness.click_row_action(file_row_id(Changes, "a.txt"), "stage-changes-a.txt", cx);
    // Only one change runs at a time.
    harness.view.read_with(cx, |view, _| {
        assert!(view.repository.as_ref().unwrap().is_busy())
    });
    harness.wait_until_idle(cx).await;
    harness
        .wait_for(
            &[(Staged, "a.txt"), (Changes, "b.txt"), (Changes, "new.txt")],
            cx,
        )
        .await;
    // The open file left its group, so the first file opens instead.
    assert_eq!(harness.open(cx).0, Some((Staged, "a.txt".into())));

    harness.click_row_action(file_row_id(Staged, "a.txt"), "unstage-staged-a.txt", cx);
    harness.wait_until_idle(cx).await;
    harness
        .wait_for(
            &[(Changes, "a.txt"), (Changes, "b.txt"), (Changes, "new.txt")],
            cx,
        )
        .await;

    // Cancelling the confirmation changes nothing.
    harness.click_row_action(file_row_id(Changes, "b.txt"), "discard-changes-b.txt", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(path.join("b.txt")).unwrap(),
        "after\n"
    );

    harness.click_row_action(file_row_id(Changes, "b.txt"), "discard-changes-b.txt", cx);
    cx.simulate_prompt_answer("放弃文件");
    harness.wait_until_idle(cx).await;
    assert_eq!(
        std::fs::read_to_string(path.join("b.txt")).unwrap(),
        "before\n"
    );

    harness.click_row_action(
        file_row_id(Changes, "new.txt"),
        "discard-changes-new.txt",
        cx,
    );
    cx.simulate_prompt_answer("删除文件");
    harness.wait_until_idle(cx).await;
    assert!(!path.join("new.txt").exists());
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
}

#[gpui_kit::test]
async fn committing_with_cmd_enter_commits_the_index(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt"]);
    let path = repository.path();
    git(path, &["add", "a.txt"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness
        .wait_for(&[(Staged, "a.txt"), (Changes, "b.txt")], cx)
        .await;

    harness.type_message("fix: a", cx);
    harness.press("cmd-enter", cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Changes, "b.txt")], cx).await;

    assert_eq!(
        git_output(path, &["log", "-1", "--format=%B"]).trim(),
        "fix: a"
    );
    assert_eq!(
        git_output(path, &["show", "--name-only", "--format=", "HEAD"]).trim(),
        "a.txt"
    );
    assert_eq!(harness.message(cx), "");
}

#[gpui_kit::test]
async fn an_empty_message_does_not_commit(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    git(path, &["add", "a.txt"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Staged, "a.txt")], cx).await;

    assert_eq!(harness.notification_count(cx), 0);
    harness.click("commit", cx);
    cx.run_until_parked();
    // Asked for a message instead of committing.
    assert_eq!(harness.notification_count(cx), 1);
    assert_eq!(
        git_output(path, &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
    harness.view.read_with(cx, |view, _| {
        assert!(!view.repository.as_ref().unwrap().is_busy())
    });
}

#[gpui_kit::test]
async fn committing_with_nothing_staged_asks_to_commit_everything(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    write(path, "new.txt", "n\n");
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "new.txt")], cx)
        .await;

    harness.type_message("everything", cx);
    harness.click("commit", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("是");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    assert_eq!(
        git_output(path, &["show", "--name-only", "--format=", "HEAD"]).trim(),
        "a.txt\nnew.txt"
    );
}

#[gpui_kit::test]
async fn a_rejected_commit_keeps_the_message(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    git(path, &["config", "core.hooksPath", ".git/hooks"]);
    write(
        path,
        ".git/hooks/pre-commit",
        "#!/bin/sh\necho no >&2\nexit 1\n",
    );
    std::process::Command::new("chmod")
        .args(["+x", ".git/hooks/pre-commit"])
        .current_dir(path)
        .status()
        .expect("chmod");
    git(path, &["add", "a.txt"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Staged, "a.txt")], cx).await;

    harness.type_message("blocked", cx);
    harness.press("cmd-enter", cx);
    harness.wait_until_idle(cx).await;

    assert_eq!(harness.notification_count(cx), 1, "the failure is reported");
    assert_eq!(harness.message(cx), "blocked");
    assert_eq!(
        git_output(path, &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[gpui_kit::test]
async fn always_commits_everything_from_then_on(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    harness.type_message("first", cx);
    harness.click("commit", cx);
    cx.simulate_prompt_answer("始终");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    assert_eq!(
        git_output(path, &["log", "-1", "--format=%s"]).trim(),
        "first"
    );
    harness.view.read_with(cx, |view, _| {
        assert!(view.settings.values().enable_smart_commit)
    });

    // Nothing staged again: committed without asking.
    write(path, "b.txt", "b\n");
    harness.press("cmd-r", cx);
    harness.wait_for(&[(Changes, "b.txt")], cx).await;
    harness.type_message("second", cx);
    harness.click("commit", cx);
    assert!(!cx.has_pending_prompt());
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    assert_eq!(
        git_output(path, &["log", "-1", "--format=%s"]).trim(),
        "second"
    );
}

#[gpui_kit::test]
async fn never_commits_only_what_is_staged(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    // In step with an upstream, so the action button stays a commit button.
    let _remote = add_remote(path);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    harness.type_message("message", cx);
    harness.click("commit", cx);
    cx.simulate_prompt_answer("从不");
    cx.run_until_parked();
    assert_eq!(
        git_output(path, &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
    harness.view.read_with(cx, |view, _| {
        assert!(!view.settings.values().suggest_smart_commit)
    });
    // Unstaged changes no longer count as something to commit.
    assert!(harness.is_disabled("commit", cx));
    harness.press("cmd-enter", cx);
    assert!(!cx.has_pending_prompt());

    git(path, &["add", "a.txt"]);
    harness.press("cmd-r", cx);
    harness.wait_for(&[(Staged, "a.txt")], cx).await;
    assert!(!harness.is_disabled("commit", cx));
}

#[gpui_kit::test]
async fn dont_show_again_syncs_without_asking_from_then_on(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    let _remote = add_remote(path);
    commit_all(path);
    let harness = open_window(Some(path.to_path_buf()), cx);
    let sync_shows = async |label: &str, harness: &Harness, cx: &mut TestAppContext| {
        cx.wait_for(harness.handle, TIMEOUT, |window, _| {
            window
                .try_find("status-sync")
                .is_some_and(|sync| sync.label() == Some(label))
        })
        .await;
    };
    sync_shows("0↓ 1↑", &harness, cx).await;

    harness.click("status-sync", cx);
    cx.simulate_prompt_answer("确定，且不再显示");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    sync_shows("0↓ 0↑", &harness, cx).await;
    harness
        .view
        .read_with(cx, |view, _| assert!(!view.settings.values().confirm_sync));

    write(path, "b.txt", "b\n");
    commit_all(path);
    harness.press("cmd-r", cx);
    sync_shows("0↓ 1↑", &harness, cx).await;
    harness.click("status-sync", cx);
    assert!(!cx.has_pending_prompt());
    harness.wait_until_idle(cx).await;
    sync_shows("0↓ 0↑", &harness, cx).await;
}

#[gpui_kit::test]
async fn the_status_bar_shows_the_branch_and_sync_state(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    assert_eq!(harness.label("status-branch", cx).as_deref(), Some("main*"));
    assert_eq!(
        harness.label("status-sync", cx).as_deref(),
        Some("发布分支")
    );

    let _remote = add_remote(path);
    write(path, "a.txt", "before\n");
    write(path, "c.txt", "c\n");
    commit_all(path);
    harness.press("cmd-r", cx);
    harness.wait_for(&[], cx).await;
    assert_eq!(harness.label("status-branch", cx).as_deref(), Some("main"));
    assert_eq!(harness.label("status-sync", cx).as_deref(), Some("0↓ 1↑"));

    // Syncing asks first; declining leaves the branch ahead.
    harness.click("status-sync", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    cx.run_until_parked();
    assert_eq!(harness.label("status-sync", cx).as_deref(), Some("0↓ 1↑"));

    harness.click("status-sync", cx);
    cx.simulate_prompt_answer("确定");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window
            .try_find("status-sync")
            .is_some_and(|sync| sync.label() == Some("0↓ 0↑"))
    })
    .await;
}

#[gpui_kit::test]
async fn a_clean_branch_off_its_upstream_offers_to_sync_changes(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let path = repository.path();
    let remote = add_remote(path);
    let harness = open_window(Some(path.to_path_buf()), cx);
    let has = |harness: &Harness, id: &'static str, cx: &mut TestAppContext| {
        harness.update(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
    };

    // Changes to commit come first, even with a commit to push.
    write(path, "b.txt", "b\n");
    commit_all(path);
    write(path, "a.txt", "again\n");
    harness.press("cmd-r", cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    assert!(has(&harness, "commit", cx));
    assert!(!has(&harness, "sync-changes", cx));
    assert!(!harness.is_disabled("commit", cx));

    // Someone else pushes too; with the tree clean the button syncs.
    let other = crate::git::fixtures::clone_of(remote.path());
    write(other.path(), "c.txt", "c\n");
    commit_all(other.path());
    git(other.path(), &["push", "--quiet"]);
    git(path, &["checkout", "--", "a.txt"]);
    git(path, &["fetch", "--quiet"]);
    harness.press("cmd-r", cx);
    harness.wait_for(&[], cx).await;
    assert!(!has(&harness, "commit", cx));
    assert_eq!(
        harness.label("sync-changes", cx).as_deref(),
        Some("同步更改 1↓ 1↑")
    );

    // Diverged, so pulling follows the user's own reconcile setting.
    git(path, &["config", "pull.rebase", "false"]);
    harness.click("sync-changes", cx);
    cx.simulate_prompt_answer("确定");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window.try_find("commit").is_some() && window.try_find("sync-changes").is_none()
    })
    .await;
    assert!(path.join("c.txt").exists());
    // In step with nothing to commit, the commit button stays but is disabled.
    assert!(harness.is_disabled("commit", cx));
}

#[gpui_kit::test]
async fn the_welcome_page_lists_recent_folders(cx: &mut TestAppContext) {
    let recent = tempfile::tempdir().expect("temporary directory");
    let repository = repository_with_changes(&["a.txt"]);
    {
        let mut folders = RecentFolders::load_from(Some(recent.path().join("recent.json")));
        for ix in 0..11 {
            folders.record(Path::new(&format!("/missing/folder{ix}")));
        }
        folders.record(&canonical(repository.path()));
    }
    let harness = open_window_with_recent(None, recent, cx);

    let name = repository
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        harness.label(("recent-folder", 0usize), cx).as_deref(),
        Some(name.as_str())
    );
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("recent-folder", 9usize)).is_some());
        assert!(window.try_find(("recent-folder", 10usize)).is_none());
    });
    harness.click("recent-more", cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("recent-folder", 11usize)).is_some());
    });

    // A folder that is gone can be forgotten.
    harness.click(("recent-folder", 1usize), cx);
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("移除");
    cx.run_until_parked();
    assert_eq!(harness.recent(cx).len(), 11);
    assert!(
        !harness
            .recent(cx)
            .contains(&PathBuf::from("/missing/folder10"))
    );

    harness.click(("recent-folder", 0usize), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    assert_eq!(harness.root(cx), Some(canonical(repository.path())));
}

#[gpui_kit::test]
async fn opening_records_the_folder_and_closing_returns_to_welcome(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt"]);
    let harness = open_window(Some(repository.path().join("")), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;
    let recorded = harness.recent(cx);
    assert_eq!(recorded.len(), 1);
    assert_eq!(canonical(&recorded[0]), canonical(repository.path()));

    harness.update(cx, |window, cx| {
        window.dispatch_action(Box::new(CloseFolder), cx)
    });
    cx.run_until_parked();
    assert_eq!(harness.root(cx), None);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("recent-folder", 0usize)).is_some());
    });
}

/// Opens the commit button's menu and chooses `item`.
fn choose_commit_menu_item(harness: &Harness, item: &str, cx: &mut TestAppContext) {
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        for label in ["提交操作", item] {
            let target = window.find_by_label(label);
            let id = target.path().last().cloned().expect("target id");
            let offset = target.bounds().center() - target.bounds().origin;
            window.click_at(id, offset, cx);
            window.render_frame(cx);
        }
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn amending_is_unavailable_before_the_first_commit(cx: &mut TestAppContext) {
    let repository = new_repository();
    let path = repository.path();
    write(path, "a.txt", "a\n");
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "a.txt")], cx).await;

    choose_commit_menu_item(&harness, "提交(修改)", cx);
    assert_eq!(harness.notification_count(cx), 0);
    harness.view.read_with(cx, |view, _| {
        assert!(!view.repository.as_ref().unwrap().is_busy())
    });

    // A disabled item leaves its menu open.
    harness.press("escape", cx);

    // Once there is a commit, the same item amends it.
    commit_all(path);
    write(path, "b.txt", "b\n");
    git(path, &["add", "b.txt"]);
    harness.press("cmd-r", cx);
    harness.wait_for(&[(Staged, "b.txt")], cx).await;
    choose_commit_menu_item(&harness, "提交(修改)", cx);
    harness.wait_until_idle(cx).await;
    assert_eq!(
        git_output(path, &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
    assert_eq!(
        git_output(path, &["show", "--name-only", "--format=", "HEAD"]).trim(),
        "a.txt\nb.txt"
    );
}

#[gpui_kit::test]
async fn stage_commit_push_and_sync_back_end_to_end(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt"]);
    let path = repository.path();
    let remote = add_remote(path);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt")], cx)
        .await;

    // Stage one file and commit it.
    harness.click_row_action(file_row_id(Changes, "a.txt"), "stage-changes-a.txt", cx);
    harness.wait_until_idle(cx).await;
    harness
        .wait_for(&[(Staged, "a.txt"), (Changes, "b.txt")], cx)
        .await;
    harness.type_message("change a", cx);
    harness.press("cmd-enter", cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Changes, "b.txt")], cx).await;
    assert_eq!(harness.label("status-sync", cx).as_deref(), Some("0↓ 1↑"));

    // Push it.
    harness.update(cx, |window, cx| window.dispatch_action(Box::new(Push), cx));
    harness.wait_until_idle(cx).await;
    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window
            .try_find("status-sync")
            .is_some_and(|sync| sync.label() == Some("0↓ 0↑"))
    })
    .await;
    let other = crate::git::fixtures::clone_of(remote.path());
    assert_eq!(
        git_output(other.path(), &["log", "-1", "--format=%s"]).trim(),
        "change a"
    );

    // Someone else pushes; fetch shows it, sync brings it back.
    write(other.path(), "c.txt", "from elsewhere\n");
    commit_all(other.path());
    git(other.path(), &["push", "--quiet"]);
    harness.update(cx, |window, cx| window.dispatch_action(Box::new(Fetch), cx));
    harness.wait_until_idle(cx).await;
    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window
            .try_find("status-sync")
            .is_some_and(|sync| sync.label() == Some("1↓ 0↑"))
    })
    .await;

    harness.click("status-sync", cx);
    cx.simulate_prompt_answer("确定");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window
            .try_find("status-sync")
            .is_some_and(|sync| sync.label() == Some("0↓ 0↑"))
    })
    .await;
    assert!(path.join("c.txt").exists());
    harness.wait_for(&[(Changes, "b.txt")], cx).await;
}
