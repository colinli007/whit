//! A file open for editing: its text in a code editor, whether it has unsaved
//! edits, and how it stays in step with the file on disk.
//!
//! Unsaved edits follow VS Code: the editor's undo stack versions the text
//! (see the gpui-base patch), and the file is dirty while that version differs
//! from the one it had when last read or saved. The file on disk is known by
//! its modification time and size, as Zed's buffers keep `saved_mtime`
//! (`crates/language/src/buffer.rs`); this is a rewrite, not a copy.
use std::{
    fs, io,
    ops::Range,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui_kit::component::{
    diff::detected_language,
    input::{EditorState, InputEvent},
};
use gpui_kit::*;

/// What the editor tab shows.
pub(crate) enum Content {
    Loading,
    /// The text, in the editor.
    Text(TextFormat),
    /// Binary or not UTF-8; VS Code shows a notice instead of the text.
    Unsupported,
    /// The file could not be read, with the system's reason.
    Failed(SharedString),
}

/// How the file's text is written: the editor holds it with `\n` line breaks
/// and no byte order mark, as Zed's buffers do, so that new lines match the
/// rest; saving writes it back in the file's own form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextFormat {
    pub crlf: bool,
    pub bom: bool,
}

/// A file's modification time and size, which change with every write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: SystemTime,
    len: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SaveError {
    /// The file changed on disk after it was last read or saved.
    Conflict,
    Io(String),
}

pub(crate) enum FileBufferEvent {
    /// The text went from saved to having unsaved edits; VS Code pins a
    /// preview tab at that moment.
    Dirtied,
}

pub(crate) struct FileBuffer {
    /// The file, in the repository or not.
    abs_path: Arc<Path>,
    /// Made with the buffer, so the tab can take focus before the file is
    /// read; it shows only while the content is text.
    editor: Entity<EditorState>,
    content: Content,
    /// The editor's version when the text last matched the file.
    saved_version: u64,
    /// The file's stamp when the text last matched it; a different stamp on
    /// disk means someone else wrote it since.
    synced: Option<Stamp>,
    /// Whether the file is gone from disk now; the tab strikes its name.
    deleted: bool,
    /// Replacing it drops a read still in flight: only the latest applies.
    load_task: Task<()>,
    _editor_subscriptions: [Subscription; 2],
}

impl EventEmitter<FileBufferEvent> for FileBuffer {}

const BOM: &str = "\u{feff}";
/// How much of a file is searched for a NUL byte, as VS Code does.
const BINARY_SNIFF_LEN: usize = 8 * 1024;
/// Past this a reload replaces larger blocks than strictly needed.
const RELOAD_DIFF_TIMEOUT: Duration = Duration::from_millis(500);

impl FileBuffer {
    pub(crate) fn open(abs_path: Arc<Path>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let language = detected_language(&abs_path.to_string_lossy());
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .folding(true)
                    // VS Code's `editor.wordWrap` is off by default; the
                    // component's is on, which lays out every line on open.
                    .soft_wrap(false)
            });
            let subscriptions = [
                // The tab's dot and the status bar's cursor position.
                cx.observe(&editor, |_, _, cx| cx.notify()),
                cx.subscribe(&editor, |this, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.edited(cx);
                    }
                }),
            ];
            let mut buffer = Self {
                abs_path,
                editor,
                content: Content::Loading,
                saved_version: 0,
                synced: None,
                deleted: false,
                load_task: Task::ready(()),
                _editor_subscriptions: subscriptions,
            };
            buffer.load(window, cx);
            buffer
        })
    }

    pub(crate) fn content(&self) -> &Content {
        &self.content
    }

    /// The editor, while the content is text.
    pub(crate) fn editor(&self) -> Option<&Entity<EditorState>> {
        matches!(self.content, Content::Text(_)).then_some(&self.editor)
    }

    /// Puts the keyboard focus in the editor, now or once the file is read.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));
    }

    pub(crate) fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub(crate) fn is_dirty(&self, cx: &App) -> bool {
        self.editor()
            .is_some_and(|editor| editor.read(cx).version() != self.saved_version)
    }

    /// Reads the whole file into the editor.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let abs_path = self.abs_path.clone();
        let read = cx.background_spawn(async move { read_file(&abs_path) });
        self.load_task = cx.spawn_in(window, async move |this, cx| {
            let read = read.await;
            let applied = this.update_in(cx, |this, window, cx| this.show(read, window, cx));
            if let Err(error) = applied {
                log::warn!("buffer released before its file was read: {error:#}");
            }
        });
    }

    /// Shows the file as read, replacing whatever the editor held.
    fn show(&mut self, read: io::Result<(Decoded, Stamp)>, window: &mut Window, cx: &mut Context<Self>) {
        match read {
            Ok((Decoded::Text { text, format }, stamp)) => {
                self.saved_version = self.editor.update(cx, |editor, cx| {
                    editor.set_value(text, window, cx);
                    editor.version()
                });
                self.content = Content::Text(format);
                self.synced = Some(stamp);
                self.deleted = false;
            }
            Ok((Decoded::Unsupported, stamp)) => {
                self.content = Content::Unsupported;
                self.synced = Some(stamp);
                self.deleted = false;
            }
            Err(error) => {
                self.deleted = error.kind() == io::ErrorKind::NotFound;
                self.content = Content::Failed(error.to_string().into());
            }
        }
        cx.notify();
    }

    /// Emits `Dirtied` for the edit that made the text differ from the file.
    fn edited(&mut self, cx: &mut Context<Self>) {
        if self.is_dirty(cx) {
            cx.emit(FileBufferEvent::Dirtied);
        }
    }

    /// Writes the text back over the file. Without `overwrite`, a file changed
    /// on disk since it was last read or saved is left alone and reported as
    /// a conflict, as VS Code does.
    pub(crate) fn save(&mut self, overwrite: bool, cx: &mut Context<Self>) -> Task<Result<(), SaveError>> {
        let Content::Text(format) = self.content else {
            return Task::ready(Ok(()));
        };
        let (version, text) = {
            let editor = self.editor.read(cx);
            (editor.version(), editor.value())
        };
        let bytes = encode(&text, format);
        let abs_path = self.abs_path.clone();
        let synced = self.synced;
        let write = cx.background_spawn(async move {
            let on_disk = stamp(&abs_path).map_err(|error| SaveError::Io(error.to_string()))?;
            // A deleted file is recreated, as VS Code does.
            let changed = on_disk.is_some() && on_disk != synced;
            if changed && !overwrite {
                return Err(SaveError::Conflict);
            }
            write_file(&abs_path, &bytes).map_err(|error| SaveError::Io(error.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let stamp = write.await?;
            this.update(cx, |this, cx| {
                this.saved_version = version;
                this.synced = Some(stamp);
                this.deleted = false;
                if let Some(editor) = this.editor() {
                    // Typing after the save is a step of its own, so undoing it
                    // returns to the saved text.
                    editor.update(cx, |editor, _| editor.break_undo_coalescing());
                }
                cx.notify();
            })
            .map_err(|error| SaveError::Io(error.to_string()))
        })
    }

    /// Follows the file after something changed it on disk: a deleted file is
    /// marked so; a changed one is reloaded unless the text has unsaved
    /// edits, which a later save reports as a conflict instead.
    pub(crate) fn disk_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let abs_path = self.abs_path.clone();
        let synced = self.synced;
        let dirty = self.is_dirty(cx);
        let version = self.editor().map(|editor| editor.read(cx).version());
        let old_text = self.editor().map(|editor| editor.read(cx).value());
        let check = cx.background_spawn(async move {
            let Some(on_disk) = stamp(&abs_path)? else {
                return Ok(DiskChange::Deleted);
            };
            // Our own write, or kept as it is because of unsaved edits.
            if Some(on_disk) == synced || dirty {
                return Ok(DiskChange::Kept);
            }
            let (decoded, on_disk) = read_file(&abs_path)?;
            let edits = match (&decoded, old_text) {
                (Decoded::Text { text, .. }, Some(old_text)) => Some(line_edits(&old_text, text)),
                _ => None,
            };
            Ok(DiskChange::Changed {
                decoded,
                on_disk,
                edits,
            })
        });
        self.load_task = cx.spawn_in(window, async move |this, cx| {
            let change: io::Result<DiskChange> = check.await;
            let applied = this.update_in(cx, |this, window, cx| {
                match change {
                    Ok(DiskChange::Deleted) => this.deleted = true,
                    Ok(DiskChange::Kept) => this.deleted = false,
                    Ok(DiskChange::Changed {
                        decoded,
                        on_disk,
                        edits,
                    }) => this.reload(decoded, on_disk, edits, version, window, cx),
                    Err(error) => log::warn!("could not re-read {}: {error:#}", this.abs_path.display()),
                }
                cx.notify();
            });
            if let Err(error) = applied {
                log::warn!("buffer released before its file was re-read: {error:#}");
            }
        });
    }

    /// Takes in the file as re-read from disk. Text that was edited while the
    /// file was read is left alone: it is dirty now, and saving will report
    /// the conflict.
    fn reload(
        &mut self,
        decoded: Decoded,
        on_disk: Stamp,
        edits: Option<Vec<(Range<usize>, String)>>,
        version: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.deleted = false;
        let editor = self.editor().cloned();
        if editor.as_ref().map(|editor| editor.read(cx).version()) != version {
            return;
        }
        match (editor, decoded, edits) {
            (Some(editor), Decoded::Text { format, .. }, Some(edits)) => {
                // One undo step that leaves the cursors on their text.
                self.saved_version = editor.update(cx, |editor, cx| {
                    editor.apply_edits(&edits, window, cx);
                    editor.version()
                });
                self.content = Content::Text(format);
                self.synced = Some(on_disk);
            }
            (_, decoded, _) => self.show(Ok((decoded, on_disk)), window, cx),
        }
    }
}

/// What a re-check of the file on disk found.
enum DiskChange {
    Deleted,
    /// The text is left as it is: the file is as last synced, or the text
    /// has unsaved edits.
    Kept,
    Changed {
        decoded: Decoded,
        on_disk: Stamp,
        /// From the text to the new content, when both are text.
        edits: Option<Vec<(Range<usize>, String)>>,
    },
}

/// A file's bytes as the editor takes them.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decoded {
    Text { text: String, format: TextFormat },
    Unsupported,
}

fn read_file(path: &Path) -> io::Result<(Decoded, Stamp)> {
    let bytes = fs::read(path)?;
    let stamp = stamp(path)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    Ok((decode(bytes), stamp))
}

/// The file's stamp, or `None` when it does not exist. Symbolic links are
/// followed: the file that matters is the one written to.
fn stamp(path: &Path) -> io::Result<Option<Stamp>> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(Stamp {
            modified: metadata.modified()?,
            len: metadata.len(),
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes over the file in place, as Zed's `RealFs::save` and VS Code do:
/// truncating keeps its inode and permissions, and writing through a symbolic
/// link keeps the link. A missing file is created along with its folders.
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<Stamp> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    stamp(path)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
}

/// Text when the bytes are UTF-8 without a NUL near the start, as the editor
/// holds it: `\n` line breaks and no byte order mark.
pub(crate) fn decode(bytes: Vec<u8>) -> Decoded {
    if bytes[..bytes.len().min(BINARY_SNIFF_LEN)].contains(&0) {
        return Decoded::Unsupported;
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Decoded::Unsupported;
    };
    let (bom, text) = match text.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    // The first line break decides, as Zed's `LineEnding::detect` does.
    let crlf = text
        .find('\n')
        .is_some_and(|newline| text[..newline].ends_with('\r'));
    Decoded::Text {
        text: text.replace("\r\n", "\n"),
        format: TextFormat { crlf, bom },
    }
}

/// The bytes to write for the editor's text: every line break in the file's
/// own form, so pasted `\r\n` text does not mix in either.
pub(crate) fn encode(text: &str, format: TextFormat) -> Vec<u8> {
    let text = text.replace("\r\n", "\n");
    let text = if format.crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    let mut bytes = Vec::with_capacity(text.len() + BOM.len());
    if format.bom {
        bytes.extend_from_slice(BOM.as_bytes());
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// The edits that turn `old` into `new`, whole lines at a time, as byte
/// ranges of `old`; unchanged lines are left alone so that cursors on them
/// stay put, as Zed's reload applies a diff.
pub(crate) fn line_edits(old: &str, new: &str) -> Vec<(Range<usize>, String)> {
    let diff = similar::TextDiff::configure()
        .timeout(RELOAD_DIFF_TIMEOUT)
        .diff_lines(old, new);
    let starts = |text: &str| -> Vec<usize> {
        std::iter::once(0)
            .chain(text.split_inclusive('\n').scan(0, |offset, line| {
                *offset += line.len();
                Some(*offset)
            }))
            .collect()
    };
    let (old_starts, new_starts) = (starts(old), starts(new));
    diff.ops()
        .iter()
        .filter(|op| op.tag() != similar::DiffTag::Equal)
        .map(|op| {
            let (old_lines, new_lines) = (op.old_range(), op.new_range());
            (
                old_starts[old_lines.start]..old_starts[old_lines.end],
                new[new_starts[new_lines.start]..new_starts[new_lines.end]].to_owned(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings GPUI's `test` macro; these are plain tests.
    use core::prelude::v1::test;

    fn text(text: &str, crlf: bool, bom: bool) -> Decoded {
        Decoded::Text {
            text: text.to_owned(),
            format: TextFormat { crlf, bom },
        }
    }

    #[test]
    fn binary_and_non_utf8_files_are_not_text() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        assert_eq!(decode(png), Decoded::Unsupported);
        // 「中文」 in GBK.
        assert_eq!(decode(vec![0xd6, 0xd0, 0xce, 0xc4]), Decoded::Unsupported);
        assert_eq!(decode(b"fn main() {}\n".to_vec()), text("fn main() {}\n", false, false));
    }

    #[test]
    fn line_breaks_and_the_byte_order_mark_round_trip() {
        let original = "\u{feff}one\r\ntwo\r\n";
        let decoded = decode(original.as_bytes().to_vec());
        assert_eq!(decoded, text("one\ntwo\n", true, true));
        let Decoded::Text { format, .. } = decoded else {
            unreachable!()
        };
        assert_eq!(encode("one\ntwo\n", format), original.as_bytes());
        // A line added in the editor ends like the others.
        assert_eq!(encode("one\nnew\ntwo\n", format), "\u{feff}one\r\nnew\r\ntwo\r\n".as_bytes());
        // Pasted CRLF text in an LF file follows the file.
        let lf = TextFormat { crlf: false, bom: false };
        assert_eq!(encode("a\r\nb\n", lf), b"a\nb\n");
    }

    #[test]
    fn the_language_follows_the_diff() {
        assert_eq!(detected_language("src/main.rs").as_ref(), "rust");
        assert_eq!(detected_language("web/app.tsx").as_ref(), "tsx");
    }

    #[test]
    fn line_edits_touch_only_the_changed_lines() {
        let old = "one\ntwo\nthree\n";
        let new = "one\nnew\ntwo\nthree\nfour\n";
        let edits = line_edits(old, new);
        assert_eq!(
            edits,
            [(4..4, "new\n".to_owned()), (14..14, "four\n".to_owned())]
        );
        // Applying them from the end gives the new text.
        let mut applied = old.to_owned();
        for (range, text) in edits.iter().rev() {
            applied.replace_range(range.clone(), text);
        }
        assert_eq!(applied, new);
        assert!(line_edits(old, old).is_empty());
        // A last line without a line break.
        assert_eq!(line_edits("a\nb", "a\nc"), [(2..3, "c".to_owned())]);
    }
}
