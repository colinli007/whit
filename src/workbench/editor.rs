//! The editor area: the tabs, the active tab's path as breadcrumbs, and its
//! content, a diff or a file to edit.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffHunkSeparator, DiffMode},
    h_flex,
    input::Editor,
    v_flex,
};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};

use super::{
    LoadState, Repository, Workbench,
    editor_group::{TabKey, display_path, home_dir},
    file_buffer::{Content, FileBuffer},
    scm_panel::{TITLE_ACTION_SIZE, icon_button},
};
use crate::{
    codicons::Codicon,
    file_icons,
    git::{ChangeGroup, StatusLetter},
    theme,
};

/// VS Code's notice for a file its editor cannot show.
pub(super) const UNSUPPORTED_FILE: &str =
    "此文件是二进制文件或使用了不支持的文本编码，所以无法在编辑器中显示。";

/// What a tab shows of itself.
struct TabLabel {
    title: SharedString,
    /// The folder, for telling apart open files of the same name.
    folder: Option<SharedString>,
    /// A diff's status letter.
    letter: Option<(&'static str, Hsla)>,
    dirty: bool,
    deleted: bool,
}

impl Workbench {
    /// The editor area. With no folder open and no tab, it shows the
    /// welcome page, as Zed's empty pane does (`crates/workspace/src/
    /// pane.rs`, the welcome page in the placeholder of a pane without
    /// worktrees).
    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let repository = self.repository.as_ref();
        let active = self.editors.group.active().cloned();
        if repository.is_none() && active.is_none() {
            return self.render_welcome(cx).into_any_element();
        }
        let (danger, muted, background) = {
            let theme = cx.theme();
            (theme.danger, theme.muted_foreground, theme.background)
        };
        let message = repository.and_then(|repository| {
            match (&repository.load_state, &active) {
                (_, Some(TabKey::File(_) | TabKey::Settings)) => None,
                (LoadState::Failed(message), _) => Some((message.clone(), danger)),
                (LoadState::Loading, _) if repository.snapshot.is_none() => {
                    Some(("正在读取改动…".into(), muted))
                }
                (_, None) if repository.files().is_empty() => {
                    Some(("没有未提交的改动".into(), muted))
                }
                (_, None) => Some(("在左侧选择一个文件".into(), muted)),
                _ => None,
            }
        });
        let file = self
            .editors
            .active_buffer()
            .map(|buffer| render_file(buffer, cx));
        let settings = matches!(active, Some(TabKey::Settings))
            .then(|| self.editors.settings.as_ref())
            .flatten()
            .map(|open| open.editor.clone());

        v_flex()
            .size_full()
            .bg(background)
            .when_some(active, |this, active| {
                let path = self.tab_path(&active);
                // The settings page has no path to show.
                let breadcrumbs = !matches!(active, TabKey::Settings);
                this.child(self.render_tab_bar(&active, cx))
                    .when(breadcrumbs, |this| this.child(render_breadcrumbs(&path, cx)))
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    // The diff stays mounted even while a file tab or nothing
                    // is shown: it measures its viewport while painting and
                    // asks for a relayout from there, which GPUI drops
                    // mid-draw. Mounting it only once a diff tab is active can
                    // leave it blank until some unrelated event redraws.
                    .when_some(repository, |this, repository| {
                        this.child(
                            Diff::new(&repository.diff)
                                .size_full()
                                .header_visible(false)
                                .hunk_separator(DiffHunkSeparator::LineInfo)
                                .change_colors(theme::colors::diff())
                                // VS Code's macOS line height: the font size times 1.5.
                                .line_height_ratio(1.5)
                                .guides(theme::colors::guides())
                                .overview_ruler(true)
                                .border_0(),
                        )
                    })
                    .when_some(file, |this, file| {
                        this.child(div().absolute().inset_0().bg(background).child(file))
                    })
                    .when_some(settings, |this, settings| {
                        this.child(div().absolute().inset_0().bg(background).child(settings))
                    })
                    .when_some(message, |this, (message, color)| {
                        this.child(centered_message("diff-message", message, color, background))
                    }),
            )
            .into_any_element()
    }

    /// How a tab's path shows: a diff's as git names it, a file's relative
    /// to the repository or from `~` outside it.
    fn tab_path(&self, key: &TabKey) -> SharedString {
        match key {
            TabKey::Diff { path, .. } => path.clone(),
            TabKey::Settings => "设置".into(),
            TabKey::File(path) => {
                let root = self.repository.as_ref().map(|repository| repository.root.as_path());
                display_path(path, root, home_dir().as_deref()).into()
            }
        }
    }

    fn render_tab_bar(&self, active: &TabKey, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tab_bar = theme.tab_bar;
        let tabs: Vec<AnyElement> = self
            .editors
            .group
            .tabs()
            .iter()
            .map(|key| {
                let label = self.tab_label(key, cx);
                self.render_tab(key, key == active, label, cx)
            })
            .collect();
        let tools = match active {
            TabKey::Diff { .. } => self
                .repository
                .as_ref()
                .map(|repository| self.render_diff_tools(repository, cx)),
            TabKey::Settings => Some(render_settings_tools(cx)),
            TabKey::File(_) => None,
        };

        h_flex()
            .h(px(35.))
            .flex_none()
            .bg(tab_bar)
            .child(
                h_flex()
                    .id("editor-tabs")
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.editors.tab_scroll)
                    .children(tabs),
            )
            .children(tools)
            .into_any_element()
    }

    fn render_tab(
        &self,
        key: &TabKey,
        is_active: bool,
        label: TabLabel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (background, foreground) = if is_active {
            (theme.tab_active, theme.tab_active_foreground)
        } else {
            (theme.tab, theme.tab_foreground)
        };
        let muted = theme.muted_foreground;
        let hovered = self.editors.hovered_tab.as_ref() == Some(key);
        let id = tab_id(key);
        let id = id.as_ref();
        let path = self.tab_path(key);
        let icon = match key {
            TabKey::Settings => Icon::new(Lucide::Settings).size(px(16.)).into_any_element(),
            _ => img(file_icons::icon_for(&path))
                .size(px(16.))
                .flex_none()
                .into_any_element(),
        };
        let tooltip: SharedString = if label.deleted {
            format!("{path} (已删除)").into()
        } else {
            path
        };
        // VS Code's dirty dot takes the close button's place until the
        // pointer is over the tab.
        let slot = if label.dirty && !hovered {
            div()
                .id(SharedString::from(format!("{id}-dirty")))
                .test_support()
                .size(px(8.))
                .rounded_full()
                .bg(foreground)
                .into_any_element()
        } else if is_active || hovered {
            let close = key.clone();
            Button::new(SharedString::from(format!("{id}-close")))
                .ghost()
                .xsmall()
                .icon(Icon::new(Lucide::X))
                .tooltip("关闭")
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.request_close_tab(close.clone(), window, cx)
                }))
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let hover_key = key.clone();
        let click_key = key.clone();
        let middle_key = key.clone();

        h_flex()
            .id(SharedString::from(id.to_owned()))
            .test_support()
            .aria_label(label.title.clone())
            .h_full()
            .flex_none()
            .pl(px(10.))
            .pr(px(4.))
            .gap(px(6.))
            .bg(background)
            .text_color(foreground)
            .border_r_1()
            .border_color(theme::colors::tab_border())
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let editors = &mut this.editors;
                let now = hovered.then(|| hover_key.clone());
                if editors.hovered_tab != now
                    && (*hovered || editors.hovered_tab.as_ref() == Some(&hover_key))
                {
                    editors.hovered_tab = now;
                    cx.notify();
                }
            }))
            // A double click keeps a preview tab, as in VS Code.
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.pin_tab(&click_key, cx);
                }
                this.activate_tab(&click_key, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| {
                    this.request_close_tab(middle_key.clone(), window, cx)
                }),
            )
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .child(icon)
            .child(
                div()
                    .when(self.editors.group.is_preview(key), |this| this.italic())
                    .when(label.deleted, |this| this.line_through())
                    .child(label.title),
            )
            .when_some(label.folder, |this, folder| {
                this.child(
                    div()
                        .id(SharedString::from(format!("{id}-folder")))
                        .test_support()
                        .aria_label(folder.clone())
                        .text_size(px(12.))
                        .text_color(muted)
                        .child(folder),
                )
            })
            .when_some(label.letter, |this, (letter, color)| {
                this.child(div().text_size(px(12.)).text_color(color).child(letter))
            })
            .child(
                h_flex()
                    .size(px(20.))
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .child(slot),
            )
            .into_any_element()
    }

    fn tab_label(&self, key: &TabKey, cx: &App) -> TabLabel {
        let title = |key: &TabKey| -> SharedString {
            let path = self.tab_path(key);
            let name = path.rsplit('/').next().unwrap_or_default().to_owned();
            match key {
                TabKey::File(_) | TabKey::Settings => name.into(),
                TabKey::Diff { group, .. } => {
                    let side = match group {
                        ChangeGroup::Staged => "索引",
                        ChangeGroup::Changes | ChangeGroup::Merge => "工作树",
                    };
                    format!("{name} ({side})").into()
                }
            }
        };
        let own = title(key);
        // VS Code adds the folder when another open tab has the same label.
        let folder = self
            .editors
            .group
            .tabs()
            .iter()
            .any(|other| other != key && title(other) == own)
            .then(|| {
                self.tab_path(key)
                    .rsplit_once('/')
                    .map(|(folder, _)| folder.to_owned().into())
            })
            .flatten();
        let (letter, dirty, deleted) = match key {
            TabKey::Diff { group, path } => (
                self.repository
                    .as_ref()
                    .and_then(|repository| repository.file(*group, path))
                    .map(|changed| (changed.letter.as_str(), changed.letter.color())),
                false,
                false,
            ),
            TabKey::Settings => (None, false, false),
            TabKey::File(path) => {
                let buffer = self.editors.buffers.get(path).map(|open| open.buffer.read(cx));
                (
                    None,
                    buffer.is_some_and(|buffer| buffer.is_dirty(cx)),
                    buffer.is_some_and(FileBuffer::is_deleted),
                )
            }
        };
        TabLabel {
            title: own,
            folder,
            letter,
            dirty,
            deleted,
        }
    }

    fn render_diff_tools(&self, repository: &Repository, cx: &mut Context<Self>) -> AnyElement {
        let split = repository.diff.read(cx).mode() == DiffMode::Split;
        // As on the change's row, a deleted file has nothing to open.
        let open_path = repository
            .diff_of(self.editors.group.active())
            .filter(|changed| changed.letter != StatusLetter::Deleted)
            .map(|changed| changed.file.path().clone());
        editor_actions()
            .when_some(open_path, |this, path| {
                this.child(
                    title_action("open-file", Codicon::GoToFile, "打开文件").on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.open_changed_file(&path, window, cx)
                        }),
                    ),
                )
            })
            .child(
                title_action("previous-change", Codicon::ArrowUp, "上一处更改")
                    .on_click(cx.listener(|this, _, _, cx| this.move_to_change(false, cx))),
            )
            .child(
                title_action("next-change", Codicon::ArrowDown, "下一处更改")
                    .on_click(cx.listener(|this, _, _, cx| this.move_to_change(true, cx))),
            )
            .child(
                title_action(
                    "toggle-inline",
                    if split {
                        Codicon::SplitVertical
                    } else {
                        Codicon::SplitHorizontal
                    },
                    if split {
                        "切换为内联视图"
                    } else {
                        "切换为并排视图"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_inline(cx))),
            )
            .into_any_element()
    }

    fn move_to_change(&mut self, next: bool, cx: &mut Context<Self>) {
        if let Some(repository) = self.repository.as_ref() {
            repository.diff.update(cx, |diff, cx| {
                if next {
                    diff.next_change(cx)
                } else {
                    diff.previous_change(cx)
                }
            });
        }
    }

    fn toggle_inline(&mut self, cx: &mut Context<Self>) {
        if let Some(repository) = self.repository.as_ref() {
            repository.diff.update(cx, |diff, cx| {
                let mode = match diff.mode() {
                    DiffMode::Split => DiffMode::Unified,
                    _ => DiffMode::Split,
                };
                diff.set_mode(mode, cx)
            });
        }
    }
}

/// The element id of a tab, by what it shows.
pub(super) fn tab_id(key: &TabKey) -> SharedString {
    match key {
        TabKey::File(path) => format!("tab-file-{}", path.display()).into(),
        TabKey::Diff { group, path } => format!("tab-{}-{path}", group.key()).into(),
        TabKey::Settings => "tab-settings".into(),
    }
}

/// The settings tab's button: VS Code's 「打开设置(JSON)」, its
/// `preferences-open-settings` icon being `go-to-file`.
fn render_settings_tools(cx: &mut Context<Workbench>) -> AnyElement {
    editor_actions()
        .child(
            title_action("settings-open-json", Codicon::GoToFile, "打开设置(JSON)").on_click(
                cx.listener(|this, _, window, cx| {
                    this.open_user_settings_json(&crate::OpenUserSettingsJson, window, cx)
                }),
            ),
        )
        .into_any_element()
}

/// The tab bar's actions as VS Code lays out `.editor-actions`: `padding: 0
/// 8px 0 4px`, and each `.action-item` with a 4px `margin-right`, the last
/// one's adding to the 8px.
fn editor_actions() -> Div {
    h_flex().h_full().pl(px(4.)).pr(px(12.)).gap(px(4.))
}

/// One of them: a 16px icon in a 22px square, as in every VS Code title bar.
fn title_action(id: &'static str, icon: impl Into<Icon>, tooltip: &'static str) -> Button {
    icon_button(id, icon, TITLE_ACTION_SIZE, false).tooltip(tooltip)
}

/// A file tab's content: its editor, or why there is none.
fn render_file(buffer: &Entity<FileBuffer>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (muted, danger, background) = (theme.muted_foreground, theme.danger, theme.background);
    let buffer = buffer.read(cx);
    match buffer.content() {
        Content::Loading => div().into_any_element(),
        Content::Text(_) => Editor::new(buffer.editor().expect("text has an editor"))
            .size_full()
            .bordered(false)
            .appearance(false)
            .into_any_element(),
        Content::Unsupported => {
            centered_message("file-message", UNSUPPORTED_FILE.into(), muted, background)
        }
        Content::Failed(reason) => {
            centered_message("file-message", reason.clone(), danger, background)
        }
    }
}

fn centered_message(
    id: &'static str,
    message: SharedString,
    color: Hsla,
    background: Hsla,
) -> AnyElement {
    v_flex()
        .id(id)
        .test_support()
        .aria_label(message.clone())
        .absolute()
        .inset_0()
        .items_center()
        .justify_center()
        .px(px(20.))
        .bg(background)
        .text_color(color)
        .child(message)
        .into_any_element()
}

/// The open file's path, folder by folder, as VS Code's breadcrumbs.
fn render_breadcrumbs(path: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len() - 1;
    let icon = file_icons::icon_for(path);
    h_flex()
        .id("breadcrumbs")
        .test_support()
        .aria_label(SharedString::from(path.to_owned()))
        .h(px(22.))
        .flex_none()
        .px(px(12.))
        .gap(px(2.))
        .text_size(px(12.))
        .text_color(theme.muted_foreground)
        .children(segments.into_iter().enumerate().map(move |(ix, segment)| {
            h_flex()
                .gap(px(2.))
                .when(ix == last, |this| {
                    this.child(img(icon.clone()).size(px(16.)))
                })
                .child(segment.to_owned())
                .when(ix < last, |this| {
                    this.child(Icon::new(IconName::ChevronRight).size(px(14.)))
                })
        }))
        .into_any_element()
}
