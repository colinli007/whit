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
use whit::{AppAssets, RecentFolders, Settings, Workbench};

fn main() {
    let scenarios: &[(&str, fn())] = &[
        ("checkout_picker", checkout_picker),
        ("checkout_over_local_changes", checkout_over_local_changes),
        ("editor_look", editor_look),
        ("in_step_with_upstream", in_step_with_upstream),
        ("narrow_side_bar", narrow_side_bar),
        ("popups", popups),
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
        let folder = folder.to_path_buf();
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(-10000.), px(-10000.)),
                            size: size(px(1280.), px(800.)),
                        })),
                        focus: false,
                        show: true,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        cx.new(|cx| {
                            Workbench::new(
                                Some(folder),
                                RecentFolders::load_from(Some(recent_file)),
                                Settings::load_from(Some(settings_file)),
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

    fn has(&mut self, id: &'static str) -> bool {
        self.window(|window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
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
