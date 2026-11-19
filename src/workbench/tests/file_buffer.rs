//! A file open for editing: unsaved edits, saving and following the file on
//! disk (`file-editor` and `file-watching`).
use std::{fs, os::unix::fs::PermissionsExt as _};

use gpui_kit::component::input::{Editor, EditorState, RopeExt as _};

use super::*;
use crate::workbench::file_buffer::{FileBuffer, SaveError};

/// A window that shows one buffer's editor, so typing and undo go through
/// the real key bindings.
struct Host(Entity<FileBuffer>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .children(self.0.read(cx).editor().map(|editor| Editor::new(editor).size_full()))
    }
}

struct Opened {
    handle: AnyWindowHandle,
    buffer: Entity<FileBuffer>,
    root: TempDir,
}

/// Opens `path` under a fresh folder holding `files`.
fn open(files: &[(&str, &[u8])], path: &str, cx: &mut TestAppContext) -> Opened {
    let root = tempfile::tempdir().expect("temporary directory");
    for (name, content) in files {
        write(root.path(), name, content);
    }
    let path: Arc<Path> = root.path().join(path).into();
    let mut buffer = None;
    let (handle, _) = cx.update(|cx| {
        crate::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let opened = FileBuffer::open(path, window, cx);
            buffer = Some(opened.clone());
            cx.new(|_| Host(opened))
        })
        .expect("open window")
    });
    cx.run_until_parked();
    Opened {
        handle,
        buffer: buffer.expect("buffer"),
        root,
    }
}

impl Opened {
    fn update<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        cx.update_window(self.handle, |_, window, cx| f(window, cx))
            .expect("window")
    }

    fn editor(&self, cx: &TestAppContext) -> Entity<EditorState> {
        self.buffer
            .read_with(cx, |buffer, _| buffer.editor().cloned())
            .expect("a text editor")
    }

    fn text(&self, cx: &TestAppContext) -> String {
        self.editor(cx).read_with(cx, |editor, _| editor.value().to_string())
    }

    fn cursor(&self, cx: &TestAppContext) -> usize {
        self.editor(cx).read_with(cx, |editor, _| editor.cursor())
    }

    fn dirty(&self, cx: &TestAppContext) -> bool {
        self.buffer.read_with(cx, |buffer, cx| buffer.is_dirty(cx))
    }

    fn deleted(&self, cx: &TestAppContext) -> bool {
        self.buffer.read_with(cx, |buffer, _| buffer.is_deleted())
    }

    fn on_disk(&self, name: &str) -> Vec<u8> {
        fs::read(self.root.path().join(name)).expect("read file")
    }

    /// Types or presses keys in the focused editor.
    fn keys(&self, input: Option<&str>, key: Option<&str>, cx: &mut TestAppContext) {
        let editor = self.editor(cx);
        self.update(cx, |window, cx| {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            window.render_frame(cx);
            if let Some(text) = input {
                window.input(text, cx);
            }
            if let Some(key) = key {
                window.press(key, cx);
            }
        });
        cx.run_until_parked();
    }

    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        self.keys(Some(text), None, cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.keys(None, Some(key), cx);
    }

    fn move_cursor_to(&self, offset: usize, cx: &mut TestAppContext) {
        let editor = self.editor(cx);
        self.update(cx, |window, cx| {
            editor.update(cx, |editor, cx| {
                let position = editor.text().offset_to_position(offset);
                editor.set_cursor_position(position, window, cx);
            })
        });
    }

    fn save(&self, overwrite: bool, cx: &mut TestAppContext) -> Result<(), SaveError> {
        let task = self
            .buffer
            .update(cx, |buffer, cx| buffer.save(overwrite, cx));
        cx.run_until_parked();
        cx.foreground_executor()
            .block_test(task)
    }

    fn disk_changed(&self, cx: &mut TestAppContext) {
        let buffer = self.buffer.clone();
        self.update(cx, |window, cx| {
            buffer.update(cx, |buffer, cx| buffer.disk_changed(window, cx))
        });
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn undoing_back_to_the_saved_text_is_clean(cx: &mut TestAppContext) {
    let opened = open(&[("a.rs", b"fn a() {}\n")], "a.rs", cx);
    assert!(!opened.dirty(cx));

    opened.type_text("x", cx);
    assert!(opened.dirty(cx));
    opened.press("cmd-z", cx);
    assert_eq!(opened.text(cx), "fn a() {}\n");
    assert!(!opened.dirty(cx));
}

#[gpui_kit::test]
fn typing_back_the_same_text_is_still_dirty(cx: &mut TestAppContext) {
    let opened = open(&[("a.rs", b"fn a() {}\n")], "a.rs", cx);
    opened.type_text("x", cx);
    opened.press("backspace", cx);
    assert_eq!(opened.text(cx), "fn a() {}\n");
    assert!(opened.dirty(cx));
}

#[gpui_kit::test]
fn typing_after_a_save_undoes_back_to_the_saved_text(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"")], "a.txt", cx);
    opened.type_text("abc", cx);
    opened.save(false, cx).expect("save");
    assert!(!opened.dirty(cx));
    assert_eq!(opened.on_disk("a.txt"), b"abc");

    opened.type_text("def", cx);
    opened.press("cmd-z", cx);
    assert_eq!(opened.text(cx), "abc");
    assert!(!opened.dirty(cx));
}

#[gpui_kit::test]
fn saving_keeps_line_breaks_and_the_byte_order_mark(cx: &mut TestAppContext) {
    let original = "\u{feff}one\r\ntwo\r\n";
    let opened = open(&[("win.txt", original.as_bytes())], "win.txt", cx);
    assert_eq!(opened.text(cx), "one\ntwo\n");

    // Edit and change back, then save: the bytes are the original ones.
    opened.type_text("x", cx);
    opened.press("backspace", cx);
    opened.save(false, cx).expect("save");
    assert_eq!(opened.on_disk("win.txt"), original.as_bytes());

    // A new line ends with CRLF like the rest.
    opened.move_cursor_to(4, cx);
    opened.type_text("new", cx);
    opened.press("enter", cx);
    opened.save(false, cx).expect("save");
    assert_eq!(
        String::from_utf8(opened.on_disk("win.txt")).unwrap(),
        "\u{feff}one\r\nnew\r\ntwo\r\n"
    );
}

#[gpui_kit::test]
fn saving_keeps_the_permissions_and_symbolic_links(cx: &mut TestAppContext) {
    let opened = open(&[("run.sh", b"echo hi\n")], "link.sh", cx);
    let root = opened.root.path();
    let script = root.join("run.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("run.sh", root.join("link.sh")).unwrap();
    // Opened before the link existed: open it again now.
    let opened = Opened {
        buffer: opened.update(cx, |window, cx| {
            FileBuffer::open(root.join("link.sh").into(), window, cx)
        }),
        ..opened
    };
    cx.run_until_parked();
    opened.update(cx, |window, cx| {
        let editor = opened.buffer.read(cx).editor().cloned().expect("text");
        editor.update(cx, |editor, cx| editor.set_value("echo bye\n", window, cx));
    });

    opened.save(false, cx).expect("save");
    let root = opened.root.path();
    assert_eq!(fs::read_to_string(root.join("run.sh")).unwrap(), "echo bye\n");
    assert!(
        fs::symlink_metadata(root.join("link.sh"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let mode = fs::metadata(root.join("run.sh")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[gpui_kit::test]
fn saving_a_read_only_file_fails(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"a\n")], "a.txt", cx);
    let path = opened.root.path().join("a.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    opened.type_text("x", cx);

    let saved = opened.save(false, cx);
    assert!(matches!(saved, Err(SaveError::Io(_))), "{saved:?}");
    assert!(opened.dirty(cx));
    assert_eq!(fs::read(&path).unwrap(), b"a\n");
}

#[gpui_kit::test]
fn a_file_changed_on_disk_is_a_conflict_until_overwritten(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"mine\n")], "a.txt", cx);
    opened.type_text("x", cx);
    write(opened.root.path(), "a.txt", "theirs, longer\n");

    assert_eq!(opened.save(false, cx), Err(SaveError::Conflict));
    assert_eq!(opened.on_disk("a.txt"), b"theirs, longer\n");
    assert!(opened.dirty(cx));

    opened.save(true, cx).expect("overwrite");
    assert_eq!(opened.on_disk("a.txt"), b"xmine\n");
    assert!(!opened.dirty(cx));
}

#[gpui_kit::test]
fn our_own_save_does_not_reload(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"one\ntwo\n")], "a.txt", cx);
    opened.move_cursor_to(4, cx);
    opened.type_text("x", cx);
    opened.save(false, cx).expect("save");
    let version = opened.editor(cx).read_with(cx, |editor, _| editor.version());

    opened.disk_changed(cx);
    assert_eq!(opened.editor(cx).read_with(cx, |editor, _| editor.version()), version);
    assert_eq!(opened.cursor(cx), 5);
}

#[gpui_kit::test]
fn a_clean_file_reloads_and_keeps_the_cursor_on_its_text(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"one\ntwo\nthree\n")], "a.txt", cx);
    // Before `three`.
    opened.move_cursor_to(8, cx);
    write(opened.root.path(), "a.txt", "one\nnew\ntwo\nthree\nfour\n");

    opened.disk_changed(cx);
    assert_eq!(opened.text(cx), "one\nnew\ntwo\nthree\nfour\n");
    assert!(!opened.dirty(cx));
    assert_eq!(opened.cursor(cx), 12, "still before `three`");

    // The reload is one undo step; undoing it leaves unsaved edits.
    opened.press("cmd-z", cx);
    assert_eq!(opened.text(cx), "one\ntwo\nthree\n");
    assert!(opened.dirty(cx));
}

#[gpui_kit::test]
fn a_dirty_file_is_not_reloaded(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"one\n")], "a.txt", cx);
    opened.type_text("x", cx);
    write(opened.root.path(), "a.txt", "theirs\n");

    opened.disk_changed(cx);
    assert_eq!(opened.text(cx), "xone\n");
    assert!(opened.dirty(cx));
}

#[gpui_kit::test]
fn a_deleted_file_is_marked_and_saving_recreates_it(cx: &mut TestAppContext) {
    let opened = open(&[("a.txt", b"one\n")], "a.txt", cx);
    fs::remove_file(opened.root.path().join("a.txt")).unwrap();

    opened.disk_changed(cx);
    assert!(opened.deleted(cx));
    assert_eq!(opened.text(cx), "one\n");

    opened.save(false, cx).expect("save");
    assert_eq!(opened.on_disk("a.txt"), b"one\n");
    assert!(!opened.deleted(cx));
}

#[gpui_kit::test]
fn binary_files_are_not_edited(cx: &mut TestAppContext) {
    let opened = open(&[("logo.png", b"\x89PNG\r\n\x1a\n\0\0\0\r")], "logo.png", cx);
    opened.buffer.read_with(cx, |buffer, _| {
        assert!(matches!(
            buffer.content(),
            crate::workbench::file_buffer::Content::Unsupported
        ));
    });
}

/// How large files open and take typing (design: Risks). Slow to set up,
/// so run on demand:
/// `cargo test --release --lib large_file -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn large_file_open_and_typing(cx: &mut TestAppContext) {
    use std::time::Instant;
    for megabytes in [1, 2, 5, 10, 20, 50] {
        let mut json = String::from("[\n");
        let mut ix = 0;
        while json.len() < megabytes * 1024 * 1024 {
            json.push_str(&format!(
                "  {{\"id\": {ix}, \"name\": \"item {ix}\", \"tags\": [\"a\", \"b\"], \"ok\": true}},\n"
            ));
            ix += 1;
        }
        json.push_str("  {}\n]\n");
        let lines = json.lines().count();

        let started = Instant::now();
        let opened = open(&[("big.json", json.as_bytes())], "big.json", cx);
        // Up to the first frame, which lays out and highlights the view.
        opened.update(cx, |window, cx| window.render_frame(cx));
        let open_time = started.elapsed();
        let mut typing = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            opened.type_text("x", cx);
            opened.update(cx, |window, cx| window.render_frame(cx));
            typing.push(started.elapsed().as_millis());
        }
        println!("{megabytes} MB, {lines} lines: open {open_time:?}, keystrokes (ms) {typing:?}");
    }
}

/// The same 50 MB as plain text, without highlighting or folding, as VS
/// Code's `editor.largeFileOptimizations` opens it; measured to choose a
/// course for large files.
/// `cargo test --release --lib large_plain -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn large_plain_text_open_and_typing(cx: &mut TestAppContext) {
    use std::time::Instant;
    let mut json = String::from("[\n");
    let mut ix = 0;
    while json.len() < 50 * 1024 * 1024 {
        json.push_str(&format!(
            "  {{\"id\": {ix}, \"name\": \"item {ix}\", \"tags\": [\"a\", \"b\"], \"ok\": true}},\n"
        ));
        ix += 1;
    }
    let mut editor = None;
    let (handle, _) = cx.update(|cx| {
        crate::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let state = cx.new(|cx| EditorState::new(window, cx));
            editor = Some(state.clone());
            cx.new(|_| PlainHost(state))
        })
        .expect("open window")
    });
    let editor = editor.unwrap();
    // With WHIT_PROFILE=<folder>, macOS `sample` records where the time goes.
    let profile = |name: &str| {
        std::env::var("WHIT_PROFILE").ok().map(|folder| {
            std::process::Command::new("sample")
                .args([&std::process::id().to_string(), "4", "-file"])
                .arg(format!("{folder}/{name}.txt"))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("start sample")
        })
    };
    let sampling = profile("open");
    let started = Instant::now();
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |editor, cx| editor.set_value(json.clone(), window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    let open_time = started.elapsed();
    if let Some(mut sampling) = sampling {
        sampling.wait().ok();
    }
    let sampling = profile("typing");
    let mut typing = Vec::new();
    let keystrokes = if sampling.is_some() { 60 } else { 5 };
    for _ in 0..keystrokes {
        let started = Instant::now();
        cx.update_window(handle, |_, window, cx| {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            window.render_frame(cx);
            window.input("x", cx);
            window.render_frame(cx);
        })
        .unwrap();
        typing.push(started.elapsed());
    }
    if let Some(mut sampling) = sampling {
        sampling.wait().ok();
    }
    println!("plain 50 MB: open {open_time:?}, keystrokes {:?}", &typing[..5]);
}

struct PlainHost(Entity<EditorState>);

impl Render for PlainHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Editor::new(&self.0).size_full()
    }
}
