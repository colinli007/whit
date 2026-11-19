//! The activity bar and the explorer (`explorer`, and the layout rules of
//! `workbench-layout`).
use super::*;
use crate::watcher::{Batch, Change};
use explorer_panel::explorer_row_id;

impl Harness {
    fn show_explorer(&self, cx: &mut TestAppContext) {
        self.click("activity-explorer", cx);
        cx.run_until_parked();
    }

    /// The rows on screen, indented two spaces a level.
    fn explorer_rows(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.view.read_with(cx, |view, _| {
            view.repository
                .as_ref()
                .unwrap()
                .explorer
                .rows()
                .iter()
                .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
                .collect()
        })
    }

    fn explorer_selected(&self, cx: &mut TestAppContext) -> Option<String> {
        self.view.read_with(cx, |view, _| {
            view.repository.as_ref().unwrap().explorer.selected.clone()
        })
    }

    fn click_row(&self, path: &str, cx: &mut TestAppContext) {
        self.click(explorer_row_id(path), cx);
        cx.run_until_parked();
    }

    fn explorer_focused(&self, cx: &mut TestAppContext) -> bool {
        let view = self.view.clone();
        self.update(cx, |window, cx| view.read(cx).explorer_focus.is_focused(window))
    }
}

/// A repository with a small tree, everything committed but `src/main.rs`.
fn project() -> TempDir {
    let repository = new_repository();
    let path = repository.path();
    write(path, "Cargo.toml", "[package]\n");
    write(path, "README.md", "readme\n");
    write(path, "src/main.rs", "fn main() {}\n");
    write(path, "src/git/read.rs", "// read\n");
    write(path, "src/git/mod.rs", "mod read;\n");
    commit_all(path);
    write(path, "src/main.rs", "fn main() { run() }\n");
    repository
}

#[gpui_kit::test]
async fn the_activity_bar_switches_the_side_bar(cx: &mut TestAppContext) {
    let repository = repository_with_changes(&["a.txt", "b.txt", "c.txt"]);
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "b.txt"), (Changes, "c.txt")], cx)
        .await;
    // Source Control by default.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("activity-scm").selected(), Some(true));
        assert!(window.try_find("explorer-section").is_none());
    });

    harness.show_explorer(cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("activity-explorer").selected(), Some(true));
        assert_eq!(window.find("activity-scm").selected(), Some(false));
        assert!(window.try_find("scm-group-changes").is_none());
    });
    // The number of changes stays on Source Control's icon.
    assert_eq!(harness.label("activity-scm-badge", cx).as_deref(), Some("3"));
}

#[gpui_kit::test]
async fn switching_views_keeps_the_side_bar_width(cx: &mut TestAppContext) {
    let repository = project();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "src/main.rs")], cx).await;
    let width = |id: &'static str, cx: &mut TestAppContext| {
        harness.update(cx, |window, cx| {
            window.render_frame(cx);
            window.find(id).bounds().size.width
        })
    };
    let before = width("scm-group-changes", cx);
    // Dragging the edge between the side bar and the editor widens it.
    let edge = harness.update(cx, |window, cx| {
        window.render_frame(cx);
        let bounds = window.find("scm-group-changes").bounds();
        point(bounds.right() + px(1.), bounds.top())
    });
    harness.update(cx, |window, cx| {
        window.drag(edge, edge + point(px(100.), px(0.)), cx)
    });
    cx.run_until_parked();
    let widened = width("scm-group-changes", cx);
    assert!(widened > before + px(50.), "{before:?} -> {widened:?}");

    harness.show_explorer(cx);
    assert_eq!(width("explorer-section", cx), widened);
}

#[gpui_kit::test]
async fn the_welcome_page_has_no_activity_bar(cx: &mut TestAppContext) {
    let harness = open_window(None, cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("activity-explorer").is_none());
    });
}

#[gpui_kit::test]
async fn the_tree_lists_folders_first_without_git(cx: &mut TestAppContext) {
    let repository = project();
    let path = repository.path();
    for name in ["b.rs", "A.rs", "file10.txt", "file2.txt"] {
        write(path, name, "");
    }
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "A.rs"), (Changes, "b.rs"), (Changes, "file10.txt"), (Changes, "file2.txt"), (Changes, "src/main.rs")], cx).await;
    harness.show_explorer(cx);

    assert_eq!(
        harness.explorer_rows(cx),
        ["src", "A.rs", "b.rs", "Cargo.toml", "file2.txt", "file10.txt", "README.md"]
    );
    let folder = path.file_name().unwrap().to_string_lossy().to_uppercase();
    assert_eq!(harness.label("explorer-section", cx), Some(folder));
}

#[gpui_kit::test]
async fn clicks_expand_folders_and_open_files(cx: &mut TestAppContext) {
    let repository = project();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "src/main.rs")], cx).await;
    harness.show_explorer(cx);
    // The first change opened with the repository, and its folder with it.
    assert_eq!(
        harness.explorer_rows(cx),
        ["src", "  git", "  main.rs", "Cargo.toml", "README.md"]
    );
    assert_eq!(harness.explorer_selected(cx).as_deref(), Some("src/main.rs"));

    // A click on an open folder closes it, and opens it again.
    harness.click_row("src", cx);
    assert_eq!(harness.explorer_rows(cx), ["src", "Cargo.toml", "README.md"]);
    harness.click_row("src", cx);
    assert_eq!(harness.explorer_rows(cx).len(), 5);

    // A click opens a preview and leaves the keyboard in the tree.
    harness.click_row("README.md", cx);
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("README.md")));
    assert!(harness.tabs(cx).contains(&harness.file_tab("README.md")));
    assert!(harness.explorer_focused(cx));
    harness.view.read_with(cx, |view, _| {
        assert!(view.editors.group.is_preview(&harness.file_tab("README.md")))
    });

    // A double click opens it for good and moves the keyboard to the editor.
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.double_click(explorer_row_id("Cargo.toml"), cx);
    });
    cx.run_until_parked();
    harness.view.read_with(cx, |view, _| {
        assert!(!view.editors.group.is_preview(&harness.file_tab("Cargo.toml")))
    });
    assert!(!harness.explorer_focused(cx));
    // The preview it replaced is gone.
    assert!(!harness.tabs(cx).contains(&harness.file_tab("README.md")));
}

#[gpui_kit::test]
async fn the_keyboard_moves_and_folds(cx: &mut TestAppContext) {
    let repository = project();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "src/main.rs")], cx).await;
    harness.show_explorer(cx);
    harness.click_row("src/main.rs", cx);

    harness.key("left", cx);
    assert_eq!(harness.explorer_selected(cx).as_deref(), Some("src"));
    harness.key("left", cx);
    assert_eq!(harness.explorer_rows(cx), ["src", "Cargo.toml", "README.md"]);
    harness.key("right", cx);
    assert_eq!(harness.explorer_rows(cx).len(), 5);
    harness.key("right", cx);
    assert_eq!(harness.explorer_selected(cx).as_deref(), Some("src/git"));
    harness.key("down", cx);
    harness.key("down", cx);
    assert_eq!(harness.explorer_selected(cx).as_deref(), Some("Cargo.toml"));
    harness.key("up", cx);
    assert_eq!(harness.explorer_selected(cx).as_deref(), Some("src/main.rs"));

    harness.key("space", cx);
    assert_eq!(harness.active_tab(cx), Some(harness.file_tab("src/main.rs")));
    harness.key("cmd-down", cx);
    harness.view.read_with(cx, |view, _| {
        assert!(!view.editors.group.is_preview(&harness.file_tab("src/main.rs")))
    });
}

#[gpui_kit::test]
async fn collapse_all_folds_every_folder(cx: &mut TestAppContext) {
    let repository = project();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "src/main.rs")], cx).await;
    harness.show_explorer(cx);
    harness.click_row("src/git", cx);
    assert_eq!(harness.explorer_rows(cx).len(), 7);

    harness.click_row_action("explorer-section".into(), "explorer-collapse-all", cx);
    cx.run_until_parked();
    assert_eq!(harness.explorer_rows(cx), ["src", "Cargo.toml", "README.md"]);
}

#[gpui_kit::test]
async fn the_active_file_is_revealed(cx: &mut TestAppContext) {
    let repository = project();
    write(repository.path(), "src/git/read.rs", "// changed\n");
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "src/git/read.rs"), (Changes, "src/main.rs")], cx)
        .await;

    harness.click(file_row_id(Changes, "src/git/read.rs"), cx);
    cx.run_until_parked();
    harness.show_explorer(cx);
    assert_eq!(
        harness.explorer_rows(cx),
        ["src", "  git", "    mod.rs", "    read.rs", "  main.rs", "Cargo.toml", "README.md"]
    );
    assert_eq!(harness.explorer_selected(cx).as_deref(), Some("src/git/read.rs"));
}

#[gpui_kit::test]
async fn files_in_ignored_folders_are_not_revealed(cx: &mut TestAppContext) {
    let repository = project();
    let path = repository.path();
    write(path, ".gitignore", "target/\n");
    git(path, &["add", ".gitignore"]);
    git(path, &["commit", "--quiet", "--message", "ignore"]);
    write(path, "target/out.txt", "built\n");
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[(Changes, "src/main.rs")], cx).await;
    harness.show_explorer(cx);

    let view = harness.view.clone();
    harness.update(cx, |window, cx| {
        view.update(cx, |view, cx| view.open_tab(harness.file_tab("target/out.txt"), true, window, cx))
    });
    cx.run_until_parked();
    assert!(!harness.explorer_rows(cx).iter().any(|row| row.contains("out.txt")));

    // Expanded by hand, the ignored folder shows its files.
    harness.click_row("target", cx);
    assert!(harness.explorer_rows(cx).contains(&"  out.txt".to_owned()));
}

#[gpui_kit::test]
async fn a_new_file_shows_in_its_open_folder(cx: &mut TestAppContext) {
    let repository = project();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[(Changes, "src/main.rs")], cx).await;
    harness.show_explorer(cx);

    write(repository.path(), "src/util.rs", "pub fn run() {}\n");
    let batch = Batch {
        changes: [Change::Worktree("src/util.rs".into())].into(),
        rescan: false,
    };
    let view = harness.view.clone();
    harness.update(cx, |window, cx| {
        view.update(cx, |view, cx| view.files_changed(&batch, window, cx))
    });
    cx.run_until_parked();
    assert_eq!(
        harness.explorer_rows(cx),
        ["src", "  git", "  main.rs", "  util.rs", "Cargo.toml", "README.md"]
    );
    harness.wait_for(&[(Changes, "src/main.rs"), (Changes, "src/util.rs")], cx).await;
    harness.view.read_with(cx, |view, _| {
        let repository = view.repository.as_ref().unwrap();
        assert_eq!(
            repository.decorations.get("src/util.rs"),
            Some(crate::git::StatusLetter::Untracked)
        );
    });
}

/// The explorer's cost follows what is expanded, not the repository's size:
/// a build folder of 200 000 files does not slow opening, and a folder of
/// 5000 entries rebuilds and draws within a frame. Slow to set up, so run on
/// demand: `cargo test --lib explorer_scales -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
async fn explorer_scales_with_what_is_expanded(cx: &mut TestAppContext) {
    use std::time::Instant;
    let repository = project();
    let path = repository.path();
    write(path, ".gitignore", "target/\n");
    commit_all(path);
    for folder in 0..200 {
        let dir = path.join(format!("target/debug/{folder}"));
        std::fs::create_dir_all(&dir).unwrap();
        for file in 0..1000 {
            std::fs::write(dir.join(format!("{file}.o")), "").unwrap();
        }
    }
    let big = path.join("big");
    std::fs::create_dir_all(&big).unwrap();
    for file in 0..5000 {
        std::fs::write(big.join(format!("entry{file}.txt")), "").unwrap();
    }
    commit_all(path);

    let started = Instant::now();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;
    harness.show_explorer(cx);
    let opened = started.elapsed();
    assert!(harness.explorer_rows(cx).contains(&"target".to_owned()));
    println!("opened with 200 000 ignored files: {opened:?}");

    // Read the big folder, then time the rebuild and the frame on their own.
    harness.click_row("big", cx);
    harness.click_row("big", cx);
    let view = harness.view.clone();
    let rebuild = view.update(cx, |view, _| {
        let explorer = &mut view.repository.as_mut().unwrap().explorer;
        let started = Instant::now();
        explorer.expand("big");
        started.elapsed()
    });
    assert!(harness.explorer_rows(cx).len() > 5000);
    let frame = harness.update(cx, |window, cx| {
        let started = Instant::now();
        window.render_frame(cx);
        started.elapsed()
    });
    println!("5000 entries: rebuild {rebuild:?}, frame {frame:?}");
    assert!(opened.as_secs_f32() < 3., "opening is not slowed by the build folder");
    assert!(rebuild.as_millis() <= 16);
}
