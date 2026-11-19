//! Screenshots of the real window, rendered by Metal off screen. The window
//! sits far outside every display and never takes focus, events go straight
//! into the app, and pixels are read back from the render target, so nothing
//! moves the user's pointer or keyboard focus.
//!
//! AppKit must run on the main thread, which libtest never gives a test, so
//! this target has its own `main` (`harness = false`):
//!
//! ```sh
//! cargo test --features visual-tests --test visual
//! ```
//!
//! Images land in `target/visual-shots/`.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _};
use gpui_kit::{VisualTestAppContext, platform::current_platform, *};
use tempfile::TempDir;
use whit::{AppAssets, RecentFolders, SettingsStore, Workbench};

fn main() {
    let scenarios: &[(&str, fn())] = &[
        ("checkout_picker", checkout_picker),
        ("checkout_over_local_changes", checkout_over_local_changes),
        ("editor_look", editor_look),
        ("in_step_with_upstream", in_step_with_upstream),
        ("narrow_side_bar", narrow_side_bar),
        ("popups", popups),
        ("stage_all_by_pointer", stage_all_by_pointer),
        ("tabs_and_file_editor", tabs_and_file_editor),
        ("changes_on_disk_show_up", changes_on_disk_show_up),
        ("explorer_tree", explorer_tree),
        ("edit_save_and_follow_the_disk", edit_save_and_follow_the_disk),
        ("welcome_page", welcome_page),
        ("settings_follow_the_file", settings_follow_the_file),
        ("activity_bar_locations", activity_bar_locations),
        ("settings_page", settings_page),
    ];
    for (name, scenario) in scenarios {
        println!("visual: {name}");
        scenario();
    }
    println!("visual: {} scenarios passed", scenarios.len());
}

/// The window open on a repository, off screen.
struct Visual {
    cx: VisualTestAppContext,
    handle: AnyWindowHandle,
    _recent: TempDir,
}

impl Visual {
    fn open(folder: &Path) -> Self {
        Self::start(Some(folder.to_path_buf()))
    }

    /// The window with no folder open: the welcome page.
    fn welcome() -> Self {
        Self::start(None)
    }

    fn start(folder: Option<PathBuf>) -> Self {
        Self::start_sized(folder, size(px(1280.), px(800.)))
    }

    fn start_sized(folder: Option<PathBuf>, window_size: Size<Pixels>) -> Self {
        let mut cx =
            VisualTestAppContext::with_asset_source(current_platform(false), Arc::new(AppAssets));
        cx.update(|cx| {
            whit::init(cx);
            // A native alert would open on the off-screen window, where
            // nothing can see or answer it; draw it inside the window instead.
            cx.set_prompt_builder(|_, message, detail, actions, handle, window, cx| {
                let prompt = cx.new(|cx| InWindowPrompt {
                    message: message.into(),
                    detail: detail.map(SharedString::from),
                    actions: actions
                        .iter()
                        .map(|action| action.label().clone())
                        .collect(),
                    focus: cx.focus_handle(),
                });
                handle.with_view(prompt, window, cx)
            });
        });
        // The recent-folder list and settings of a test are not the user's.
        let recent = tempfile::tempdir().expect("temporary directory");
        let recent_file = recent.path().join("recent.json");
        let settings_file = recent.path().join("settings.json");
        let (handle, _) = cx
            .update(|cx| {
                cx.set_global(SettingsStore::load_from(Some(settings_file)));
                SettingsStore::watch(cx);
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(-10000.), px(-10000.)),
                            size: window_size,
                        })),
                        focus: false,
                        show: true,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        cx.new(|cx| {
                            Workbench::new(
                                folder,
                                RecentFolders::load_from(Some(recent_file)),
                                window,
                                cx,
                            )
                        })
                    },
                )
            })
            .expect("open the off-screen window");
        let mut visual = Self {
            cx,
            handle,
            _recent: recent,
        };
        visual.settle();
        visual
    }

    /// Runs every queued task, git included, then draws a fresh frame.
    fn settle(&mut self) {
        for _ in 0..5 {
            self.cx.run_until_parked();
        }
        self.window(|window, cx| window.render_frame(cx));
    }

    /// Lets `duration` pass on both the executor's simulated clock, which
    /// timers such as a tooltip's delay follow, and the real one, which
    /// animations read, drawing a frame every 16 milliseconds.
    fn wait(&mut self, duration: Duration) {
        let frame = Duration::from_millis(16);
        let mut waited = Duration::ZERO;
        while waited < duration {
            std::thread::sleep(frame);
            self.cx.advance_clock(frame);
            self.cx.run_until_parked();
            self.window(|window, cx| window.render_frame(cx));
            waited += frame;
        }
    }

    fn window<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.handle, |_, window, cx| f(window, cx))
            .expect("the window is open")
    }

    fn click(&mut self, id: &'static str) {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        });
        self.settle();
    }

    /// Presses and releases the primary button at a window position.
    fn click_at(&mut self, position: Point<Pixels>) {
        self.window(|window, cx| window.render_frame(cx));
        self.cx
            .simulate_click(self.handle, position, Modifiers::default());
        self.settle();
    }

    /// Drags with the primary button between window positions.
    fn drag(&mut self, from: Point<Pixels>, to: Point<Pixels>) {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.drag(from, to, cx);
        });
        self.settle();
    }

    /// Moves the pointer to a window position.
    fn move_to(&mut self, position: Point<Pixels>) {
        self.window(|window, cx| window.render_frame(cx));
        self.cx
            .simulate_mouse_move(self.handle, position, None, Modifiers::default());
        self.settle();
    }

    fn hover(&mut self, id: &'static str) {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.hover(id, cx);
        });
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.input(text, cx);
        });
        self.settle();
    }

    fn press(&mut self, key: &str) {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        });
        self.settle();
    }

    fn label(&mut self, id: &'static str) -> Option<String> {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.try_find(id)?.label().map(str::to_owned)
        })
    }

    fn has(&mut self, id: impl Into<SharedString>) -> bool {
        let id = id.into();
        self.window(|window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
    }

    /// How many pixels of the code area, right of the side bar and below the
    /// tabs, differ from its top-left one: none means it is blank.
    fn drawn_code_pixels(&mut self) -> usize {
        let image = self
            .cx
            .capture_screenshot(self.handle)
            .expect("render to an image");
        let scale = image.width() as f32 / 1280.;
        let left = (420. * scale) as u32;
        let top = (90. * scale) as u32;
        let background = *image.get_pixel(left, top);
        image
            .enumerate_pixels()
            .filter(|(x, y, pixel)| *x >= left && *y >= top && **pixel != background)
            .count()
    }

    fn shot(&mut self, name: &str) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/visual-shots");
        std::fs::create_dir_all(&dir).expect("create the screenshot folder");
        let path = dir.join(format!("{name}.png"));
        self.cx
            .capture_screenshot(self.handle)
            .expect("render to an image")
            .save(&path)
            .expect("save the screenshot");
        println!("  {}", path.display());
    }
}

/// An alert drawn in the window: the message, its detail, and one button per
/// answer, each findable as `prompt-<label>`.
struct InWindowPrompt {
    message: SharedString,
    detail: Option<SharedString>,
    actions: Vec<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<PromptResponse> for InWindowPrompt {}

impl Focusable for InWindowPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for InWindowPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let card = div()
            .track_focus(&self.focus)
            .w(px(260.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .bg(rgb(0xE8E8E8))
            .text_color(rgb(0x1F1F1F))
            .rounded(px(10.))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(self.message.clone()),
            )
            .children(
                self.detail
                    .clone()
                    .map(|detail| div().text_size(px(12.)).child(detail)),
            )
            .children(self.actions.iter().enumerate().map(|(ix, label)| {
                div()
                    .id(SharedString::from(format!("prompt-{label}")))
                    .test_support()
                    .py(px(4.))
                    .flex()
                    .justify_center()
                    .rounded(px(6.))
                    .when(ix == 0, |this| {
                        this.bg(cx.theme().primary).text_color(rgb(0xFFFFFF))
                    })
                    .child(label.clone())
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(ix))))
            }));
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000066))
            .child(card)
    }
}

/// The element id of the file tab of `name` in `root`: tabs are keyed by
/// the file's real path.
fn file_tab(root: &Path, name: &str) -> String {
    let root = root.canonicalize().expect("real path");
    format!("tab-file-{}", root.join(name).display())
}

fn git(directory: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .status()
        .expect("git should run");
    assert!(status.success(), "git {arguments:?} failed");
}

/// A repository on `main` with a second branch `dev` and a local change.
fn repository() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary directory");
    let path = repository.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join("a.txt"), "1\n2\n3\n").unwrap();
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "--message", "first"]);
    // `dev` changed the first line, so a local edit of the last one blocks
    // checking it out, yet merges cleanly when migrated.
    git(path, &["checkout", "--quiet", "-b", "dev"]);
    std::fs::write(path.join("a.txt"), "one\n2\n3\n").unwrap();
    git(path, &["commit", "--quiet", "--all", "--message", "dev"]);
    git(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("a.txt"), "1\n2\nthree\n").unwrap();
    repository
}

fn checkout_picker() {
    let repository = repository();
    let mut visual = Visual::open(repository.path());
    assert!(
        visual.has("status-branch"),
        "the repository loads off screen"
    );
    visual.shot("01-repository");

    visual.click("status-branch");
    assert!(visual.has("command"), "the checkout picker opens");
    visual.shot("02-checkout");
}

fn checkout_over_local_changes() {
    let repository = repository();
    let path = repository.path();
    let mut visual = Visual::open(path);
    visual.click("status-branch");
    visual.type_text("dev");
    visual.press("enter");
    assert!(visual.has("prompt-迁移更改"), "签出会覆盖本地更改。");
    visual.shot("03-checkout-prompt");

    visual.click("prompt-迁移更改");
    assert_eq!(visual.label("status-branch").as_deref(), Some("dev*"));
    assert_eq!(
        std::fs::read_to_string(path.join("a.txt")).unwrap(),
        "one\n2\nthree\n"
    );
    visual.shot("04-migrated");
}

const GALLERY_TSX: &str = r#"import { Avatar } from "./avatar";

export const Gallery = ({ photos, size, selectable }: GalleryProps) => {
  const [selected, setSelected] = useState<number[]>([]);
  return (
    <div
      className={`grid gap-2 ${selectable ? "cursor-pointer" : ""}`}
      onPointerLeave={() => {
        setSelected([]);
      }}
    >
      <ul className="flex flex-wrap">
        {photos.map((photo, index) => (
          <li
            key={index}
            title={formatTitle(photo, size.width, size.height, true)}
            role="option"
          />
        ))}
      </ul>
      <Avatar name={DEFAULT_NAME} size={16} />
    </div>
  );
};
"#;

/// The editor area as VS Code draws a TSX diff with Monokai Dimmed.
fn editor_look() {
    let repository = tempfile::tempdir().expect("temporary directory");
    let path = repository.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join("Gallery.tsx"), GALLERY_TSX).unwrap();
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "--message", "first"]);
    std::fs::write(
        path.join("Gallery.tsx"),
        GALLERY_TSX.replace("size.height, true)", "size.height)"),
    )
    .unwrap();

    let mut visual = Visual::open(path);
    visual.click("scm-file-changes-Gallery.tsx");
    visual.shot("05-editor");

    // Past the end of `key={index}` in the right column: inside the `(` that
    // `=> (` opens, so that pair's guide turns opaque and gains its corners.
    visual.click_at(point(px(1100.), px(row_y(15))));
    visual.shot("06-active-guides");

    // A file taller than the window, changed near the top and the bottom: the
    // ruler marks both, and pressing low on it scrolls there.
    let long: String = (1..=200)
        .map(|n| format!("export const value{n} = {n};\n"))
        .collect();
    std::fs::write(path.join("long.ts"), &long).unwrap();
    git(path, &["add", "long.ts"]);
    git(path, &["commit", "--quiet", "--message", "long"]);
    std::fs::write(
        path.join("long.ts"),
        long.replace("value5 = 5;", "value5 = 50;")
            .replace("export const value190 = 190;\n", ""),
    )
    .unwrap();
    visual.click("scm-refresh");
    visual.click("scm-file-changes-long.ts");
    visual.shot("07-ruler");
    visual.click_at(point(px(1265.), px(700.)));
    visual.shot("08-ruler-scrolled");
}

/// The middle of the editor row showing line `line`, in window coordinates:
/// below the 35-pixel tab bar and 22-pixel breadcrumbs, rows of 21 pixels.
fn row_y(line: usize) -> f32 {
    35. + 22. + 21. * (line as f32 - 0.5)
}

/// Committed and pushed: VS Code shows a disabled commit button and the sync
/// icon without counts.
fn in_step_with_upstream() {
    let repository = repository();
    let path = repository.path();
    git(path, &["checkout", "--", "a.txt"]);
    let remote = tempfile::tempdir().expect("temporary directory");
    git(remote.path(), &["init", "--quiet", "--bare"]);
    git(
        path,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(
        path,
        &["push", "--quiet", "--set-upstream", "origin", "main"],
    );
    let mut visual = Visual::open(path);
    assert_eq!(visual.label("status-sync").as_deref(), Some("0↓ 0↑"));
    visual.shot("09-in-step");
}

/// The side bar dragged to the 190 pixels of the user's VS Code: the folder
/// gives way before the name, and a row's actions take room only while
/// hovered or selected.
fn narrow_side_bar() {
    let repository = tempfile::tempdir().expect("temporary directory");
    let path = repository.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    let folder = path.join("src/parser");
    std::fs::create_dir_all(&folder).unwrap();
    for name in ["lexer.ts", "token.ts"] {
        std::fs::write(folder.join(name), "export {};\n").unwrap();
    }
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "--message", "first"]);
    for name in ["lexer.ts", "token.ts"] {
        std::fs::write(folder.join(name), "export const a = 1;\n").unwrap();
    }

    let mut visual = Visual::open(path);
    visual.shot("10-side-bar-wide");
    visual.drag(point(px(300.), px(400.)), point(px(190.), px(400.)));
    visual.click("scm-file-changes-src/parser/lexer.ts");
    visual.shot("11-side-bar-narrow");
    visual.hover("scm-file-changes-src/parser/token.ts");
    assert!(
        visual.has("discard-changes-src/parser/token.ts"),
        "the hovered row shows its actions"
    );
    visual.shot("12-side-bar-narrow-hover");
}

/// The 「…」 menu with a submenu open, a button's tooltip and a toast, sized
/// as VS Code draws them.
fn popups() {
    let repository = repository();
    let mut visual = Visual::open(repository.path());
    visual.click("scm-more");
    for _ in 0..5 {
        visual.press("down");
    }
    visual.shot("13-menu");
    visual.press("escape");
    visual.press("escape");

    visual.hover("scm-refresh");
    // The tooltip waits for the pointer to rest.
    visual.wait(Duration::from_millis(1500));
    visual.shot("14-tooltip");

    // At the window's right edge the tooltip is pushed in, and its arrow
    // stays on the button.
    visual.hover("toggle-inline");
    visual.wait(Duration::from_millis(1500));
    visual.shot("14b-tooltip-at-edge");

    // Committing with no message warns.
    visual.click("commit");
    visual.wait(Duration::from_millis(800));
    visual.shot("15-toast");
}

/// The pointer goes over the Changes header, rests on 「放弃所有更改」 until
/// its tooltip shows, then moves on to 「+」 and clicks it: the file moves to
/// 「暂存的更改」 and 「更改」 stays, empty, as VS Code shows it.
fn stage_all_by_pointer() {
    let repository = repository();
    let path = repository.path();
    let mut visual = Visual::open(path);
    let center = |visual: &mut Visual, id: &'static str| {
        visual.window(|window, cx| {
            window.render_frame(cx);
            window.find(id).bounds().center()
        })
    };
    let header = center(&mut visual, "scm-group-changes");
    visual.move_to(header);
    let discard = center(&mut visual, "discard-all-changes");
    visual.move_to(discard);
    visual.wait(Duration::from_millis(1500));
    visual.shot("16-discard-all-tooltip");
    let stage = center(&mut visual, "stage-all-changes");
    println!("  header {header:?} discard {discard:?} stage {stage:?}");
    visual.move_to(stage);
    visual.wait(Duration::from_millis(300));
    visual.shot("17-over-stage-all");
    visual.click_at(stage);
    visual.wait(Duration::from_millis(300));
    visual.shot("18-after-stage-all");
    let status = Command::new("git")
        .current_dir(path)
        .args(["status", "--porcelain"])
        .output()
        .expect("git should run");
    let status = String::from_utf8_lossy(&status.stdout).into_owned();
    println!("  status {status:?}");
    assert_eq!(status, "M  a.txt\n", "「+」 stages the changes");
    assert!(
        visual.has("scm-group-changes"),
        "the emptied 「更改」 group stays as in VS Code"
    );
}

/// A diff tab and a file tab side by side; going back to the diff from the
/// file finds it drawn, not blank.
fn tabs_and_file_editor() {
    let repository = tempfile::tempdir().expect("temporary directory");
    let path = repository.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join("Gallery.tsx"), GALLERY_TSX).unwrap();
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "--message", "first"]);
    std::fs::write(
        path.join("Gallery.tsx"),
        GALLERY_TSX.replace("size.height, true)", "size.height)"),
    )
    .unwrap();

    let mut visual = Visual::open(path);
    visual.hover("scm-file-changes-Gallery.tsx");
    visual.click("open-changes-Gallery.tsx");
    let gallery_tab = file_tab(path, "Gallery.tsx");
    assert!(visual.has(gallery_tab.clone()), "the file tab opens");
    // Opened for good, the file has the keyboard.
    visual.type_text("// edited\n");
    assert!(visual.has(format!("{gallery_tab}-dirty")), "unsaved edits show a dot");
    visual.shot("19-file-tab");

    visual.click("tab-changes-Gallery.tsx");
    let drawn = visual.drawn_code_pixels();
    println!("  diff pixels after switching back: {drawn}");
    assert!(drawn > 10_000, "the diff is drawn after switching back");
    visual.shot("20-back-to-diff");
}

/// A file written by someone else shows up in the changes within a second,
/// with nothing done in the window: the real file watcher at work.
fn changes_on_disk_show_up() {
    let repository = repository();
    let mut visual = Visual::open(repository.path());
    assert!(visual.has("scm-file-changes-a.txt"));
    // FSEvents starts reporting a moment after the stream opens.
    visual.wait(Duration::from_millis(300));

    std::fs::write(repository.path().join("b.txt"), "from the terminal\n").unwrap();
    let started = std::time::Instant::now();
    while !visual.has("scm-file-changes-b.txt") {
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the new file shows within a second"
        );
        visual.wait(Duration::from_millis(50));
    }
    println!("  shown after {:?}", started.elapsed());
    visual.shot("21-changed-on-disk");
}

/// The explorer with git colors, an ignored folder and indent guides.
fn explorer_tree() {
    let repository = tempfile::tempdir().expect("temporary directory");
    let path = repository.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    let write = |name: &str, text: &str| {
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write(".gitignore", "target/\n");
    write("Cargo.toml", "[package]\n");
    write("README.md", "readme\n");
    write("src/main.rs", "fn main() {}\n");
    write("src/git/mod.rs", "mod read;\n");
    write("src/git/read.rs", "// read\n");
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "--message", "first"]);
    write("src/git/read.rs", "// changed\n");
    write("src/git/new.rs", "// new\n");
    write("target/debug/out.o", "");

    let mut visual = Visual::open(path);
    visual.click("activity-explorer");
    assert!(visual.has("explorer-src/git/read.rs"), "the first change is revealed");
    visual.hover("explorer-src/git/new.rs");
    visual.shot("22-explorer");
}

/// The whole flow of the change, off screen with the real file watcher: a
/// preview from the explorer, kept by editing, saved into the changes,
/// reloaded after a write elsewhere, a conflicting save overwritten, the
/// file deleted, and quitting asking about unsaved edits.
fn edit_save_and_follow_the_disk() {
    let repository = tempfile::tempdir().expect("temporary directory");
    let path = repository.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    let notes = path.join("notes.txt");
    std::fs::write(&notes, "one\ntwo\n").unwrap();
    std::fs::write(path.join("other.txt"), "other\n").unwrap();
    git(path, &["add", "--all"]);
    git(path, &["commit", "--quiet", "--message", "first"]);

    let mut visual = Visual::open(path);
    visual.wait(Duration::from_millis(300));
    visual.click("activity-explorer");
    visual.click("explorer-notes.txt");
    let notes_tab = file_tab(path, "notes.txt");
    let notes_dirty = format!("{notes_tab}-dirty");
    assert!(visual.has(notes_tab.clone()));
    visual.shot("23-preview");

    // Opened for good with the keyboard in the editor, then edited.
    visual.press("cmd-down");
    visual.type_text("hello ");
    assert!(visual.has(notes_dirty.clone()));
    visual.shot("24-edited");

    visual.press("cmd-s");
    visual.wait(Duration::from_millis(400));
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "hello one\ntwo\n");
    assert!(!visual.has(notes_dirty.clone()));
    assert_eq!(visual.label("activity-scm-badge").as_deref(), Some("1"), "saved into the changes");
    visual.shot("25-saved");

    // Written elsewhere while saved: reloaded without asking.
    std::fs::write(&notes, "hello one\ntwo\nthree from the terminal\n").unwrap();
    visual.wait(Duration::from_millis(400));
    assert!(!visual.has(notes_dirty.clone()));
    visual.shot("26-reloaded");

    // Edited here and there: the save asks to overwrite.
    visual.type_text("x");
    std::fs::write(&notes, "theirs\n").unwrap();
    visual.wait(Duration::from_millis(400));
    visual.press("cmd-s");
    visual.wait(Duration::from_millis(400));
    assert!(visual.has("save-overwrite"), "the conflict offers to overwrite");
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "theirs\n");
    visual.shot("27-conflict");
    visual.click("save-overwrite");
    visual.wait(Duration::from_millis(400));
    assert!(std::fs::read_to_string(&notes).unwrap().contains("three from the terminal"));
    visual.shot("28-overwritten");

    // Deleted elsewhere: the tab stays, struck through.
    std::fs::remove_file(&notes).unwrap();
    visual.wait(Duration::from_millis(400));
    assert!(visual.has(notes_tab.clone()));
    visual.shot("29-deleted");

    // Unsaved edits make quitting ask; canceling keeps everything.
    visual.type_text("y");
    visual.press("cmd-q");
    assert!(visual.has("prompt-保存"), "quitting asks about the unsaved file");
    visual.shot("30-quit-asks");
    visual.click("prompt-取消");
    assert!(visual.has(notes_dirty.clone()));
}

/// The welcome page, drawn as the empty editor area of a window with no
/// folder open.
fn welcome_page() {
    let mut visual = Visual::welcome();
    assert!(visual.has("open-folder"));
    assert!(!visual.has("activity-explorer"));
    visual.shot("31-welcome");
}

/// The settings file written in the terminal takes effect in a running
/// window within a second, through the real file watcher.
fn settings_follow_the_file() {
    let mut visual = Visual::welcome();
    let file = visual._recent.path().join("settings.json");
    let confirm_sync = |visual: &mut Visual| {
        visual
            .cx
            .update(|cx| SettingsStore::global(cx).values().confirm_sync)
    };
    assert!(confirm_sync(&mut visual));
    visual.wait(Duration::from_millis(300));

    let written = std::time::Instant::now();
    // Saved as an editor saves: another file renamed over it.
    std::fs::write(file.with_extension("tmp"), "{ \"git.confirmSync\": false }").unwrap();
    std::fs::rename(file.with_extension("tmp"), &file).unwrap();
    while confirm_sync(&mut visual) && written.elapsed() < Duration::from_secs(1) {
        visual.wait(Duration::from_millis(50));
    }
    println!("  applied after {:?}", written.elapsed());
    assert!(!confirm_sync(&mut visual), "applied within a second");
}

/// The activity bar in each of its places, set through the settings file.
fn activity_bar_locations() {
    let repository = repository();
    let mut visual = Visual::open(repository.path());
    visual.wait(Duration::from_millis(300));
    let file = visual._recent.path().join("settings.json");
    for (location, shot) in [
        ("default", "32-activity-bar-default"),
        ("top", "33-activity-bar-top"),
        ("bottom", "34-activity-bar-bottom"),
        ("hidden", "35-activity-bar-hidden"),
    ] {
        std::fs::write(
            &file,
            format!("{{ \"workbench.activityBar.location\": \"{location}\" }}"),
        )
        .unwrap();
        visual.wait(Duration::from_millis(400));
        assert_eq!(visual.has("activity-manage"), location == "default", "{location}");
        visual.shot(shot);
    }
}

/// The settings page as VS Code 1.117 draws it: full width, narrow with no
/// table of contents, filtered to the modified settings, and opened with
/// no folder.
fn settings_page() {
    let repository = repository();
    let mut visual = Visual::open(repository.path());
    std::fs::write(
        visual._recent.path().join("settings.json"),
        "{\n    \"workbench.activityBar.location\": \"top\"\n}\n",
    )
    .unwrap();
    visual.wait(Duration::from_millis(400));
    visual.press("cmd-,");
    assert!(visual.has("tab-settings"));
    assert!(visual.has("settings-toc"));
    visual.hover("setting-git.confirmSync");
    visual.shot("36-settings");

    visual.type_text("@modified");
    visual.wait(Duration::from_millis(100));
    assert_eq!(visual.label("settings-count").as_deref(), Some("找到 1 个设置"));
    visual.shot("37-settings-modified");

    // Narrower than VS Code's 700px, the table of contents goes.
    let mut narrow = Visual::start_sized(None, size(px(650.), px(800.)));
    narrow.press("cmd-,");
    assert!(narrow.has("tab-settings"));
    assert!(!narrow.has("settings-toc"));
    narrow.shot("37b-settings-narrow");

    let mut welcome = Visual::welcome();
    welcome.press("cmd-,");
    assert!(welcome.has("tab-settings"));
    welcome.shot("38-settings-without-a-folder");
}
