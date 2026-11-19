//! The explorer side bar view as VS Code draws it: the title, the folder's
//! section, and its tree with git colors (`explorer`).
use std::ops::Range;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};

use super::{
    Repository, SideView, Workbench,
    editor_group::TabKey,
    explorer::{self, DirEntry, Row},
};
use crate::{file_icons, theme::colors};

const ROW_HEIGHT: f32 = 22.;
/// `workbench.tree.indent`'s default.
const INDENT: f32 = 8.;
/// The row's padding before the first level of indent.
const ROW_INSET: f32 = 8.;
/// The folder arrow's box, `.monaco-tl-twistie`.
const TWISTIE: f32 = 16.;

/// The element id of an explorer row.
pub(super) fn explorer_row_id(path: &str) -> SharedString {
    format!("explorer-{path}").into()
}

impl Workbench {
    pub(super) fn render_explorer(
        &self,
        repository: &Repository,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (sidebar, sidebar_border) = {
            let theme = cx.theme();
            (theme.sidebar, theme.sidebar_border)
        };
        let folder = repository
            .root
            .file_name()
            .map(|name| name.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let section_id: SharedString = "explorer-section".into();
        let section_hovered = self.scm_hovered_row.as_ref() == Some(&section_id);
        let collapsed = repository.explorer_collapsed;
        let count = repository.explorer.rows().len();
        let focused = self.explorer_focus.contains_focused(window, cx);

        let title = self.side_bar_title(repository, "EXPLORER", None, cx);

        let section = h_flex()
            .id(section_id.clone())
            .test_support()
            .aria_label(SharedString::from(folder.clone()))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let id: SharedString = "explorer-section".into();
                if *hovered {
                    this.scm_hovered_row = Some(id);
                } else if this.scm_hovered_row.as_ref() == Some(&id) {
                    this.scm_hovered_row = None;
                } else {
                    return;
                }
                cx.notify();
            }))
            .h(px(ROW_HEIGHT))
            .flex_none()
            .pl(px(2.))
            .pr(px(8.))
            .gap(px(2.))
            .bg(colors::side_bar_section_header())
            .cursor_pointer()
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(16.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .child(folder),
            )
            .when(section_hovered, |this| {
                this.child(
                    Button::new("explorer-collapse-all")
                        .ghost()
                        .size(px(20.))
                        .p_0()
                        .child(Icon::new(Lucide::CopyMinus).size(px(16.)))
                        .tooltip("在资源管理器中折叠文件夹")
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.collapse_all_folders(cx)
                        })),
                )
            })
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(repository) = this.repository.as_mut() {
                    repository.explorer_collapsed = !repository.explorer_collapsed;
                    cx.notify();
                }
            }));

        let tree = (!collapsed).then(|| {
            div()
                .id("explorer-tree")
                .track_focus(&self.explorer_focus)
                .flex_1()
                .min_h_0()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.explorer_hovered = *hovered;
                    cx.notify();
                }))
                .on_key_down(cx.listener(Self::explorer_key_down))
                .child(
                    uniform_list(
                        "explorer-rows",
                        count,
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            this.render_explorer_rows(range, focused, cx)
                        }),
                    )
                    .size_full()
                    .track_scroll(&repository.explorer_scroll),
                )
        });

        v_flex()
            .size_full()
            .bg(sidebar)
            .border_r_1()
            .border_color(sidebar_border)
            .child(title)
            .child(section)
            .children(tree)
    }

    fn render_explorer_rows(
        &mut self,
        range: Range<usize>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repository) = self.repository.as_ref() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let (foreground, list_hover) = (theme.foreground, theme.list_hover);
        let selected_background = if focused {
            colors::list_focused_selection()
        } else {
            theme.list_active
        };
        let guides = self.explorer_hovered;
        repository.explorer.rows()[range]
            .iter()
            .map(|row| {
                let ignored = repository
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.is_ignored(&row.path));
                let decoration = repository.decorations.get(&row.path);
                let name_color = match decoration {
                    Some(letter) => letter.color(),
                    None if ignored => colors::ignored(),
                    None => foreground,
                };
                let selected = repository.explorer.selected.as_deref() == Some(row.path.as_str());
                self.render_explorer_row(
                    row,
                    name_color,
                    decoration.map(|letter| (letter, row.is_dir)),
                    selected.then_some((selected_background, focused)),
                    list_hover,
                    guides,
                    cx,
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_explorer_row(
        &self,
        row: &Row,
        name_color: Hsla,
        decoration: Option<(crate::git::StatusLetter, bool)>,
        selected: Option<(Hsla, bool)>,
        list_hover: Hsla,
        guides: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = if row.is_dir {
            file_icons::folder_icon(row.expanded)
        } else {
            file_icons::icon_for(&row.path)
        };
        let indent = ROW_INSET + row.depth as f32 * INDENT;
        let path = row.path.clone();
        let is_dir = row.is_dir;

        h_flex()
            .id(explorer_row_id(&row.path))
            .test_support()
            .aria_label(SharedString::from(row.name.clone()))
            .aria_selected(selected.is_some())
            .relative()
            .h(px(ROW_HEIGHT))
            .w_full()
            .pl(px(indent))
            .pr(px(12.))
            .gap(px(6.))
            .cursor_pointer()
            .map(|this| match selected {
                Some((background, _)) => this.bg(background),
                None => this.hover(move |style| style.bg(list_hover)),
            })
            // `list.focusOutline` inside the row, as in the source control view.
            .when(selected.is_some_and(|(_, focused)| focused), |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .border_1()
                        .border_color(colors::list_focus_outline()),
                )
            })
            // Indent guides through each level, shown while the pointer is
            // over the tree as VS Code's `renderIndentGuides: onHover`.
            .when(guides, |this| {
                this.children((0..row.depth).map(|level| {
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(ROW_INSET + level as f32 * INDENT + TWISTIE / 2.))
                        .w(px(1.))
                        .bg(colors::tree_indent_guide())
                }))
            })
            .child(
                div()
                    .w(px(TWISTIE))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .when(is_dir, |this| {
                        this.child(
                            Icon::new(if row.expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(16.)),
                        )
                    }),
            )
            .child(img(icon).size(px(16.)).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(name_color)
                    .child(row.name.clone()),
            )
            .when_some(decoration, |this, (letter, is_dir)| {
                // Folders carry a dot of their most urgent change, files the
                // letter.
                this.child(
                    div()
                        .w(px(12.))
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(letter.color())
                        .child(if is_dir { "●" } else { letter.as_str() }),
                )
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                window.focus(&this.explorer_focus, cx);
                this.explorer_clicked(path.clone(), is_dir, event.click_count(), window, cx)
            }))
            .into_any_element()
    }

    pub(super) fn show_side_view(&mut self, view: SideView, cx: &mut Context<Self>) {
        if let Some(repository) = self.repository.as_mut() {
            repository.side_view = view;
            cx.notify();
        }
    }

    /// A click selects the row: a folder opens or closes, a file opens in a
    /// preview tab, or for good on a double click, which also moves the
    /// keyboard into the editor.
    fn explorer_clicked(
        &mut self,
        path: String,
        is_dir: bool,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        repository.explorer.selected = Some(path.clone());
        if is_dir {
            if clicks == 1 {
                self.toggle_folder(&path, cx);
            }
        } else if let Some(key) = self.file_key(&path) {
            self.open_tab(key, clicks < 2, window, cx);
        }
        cx.notify();
    }

    fn toggle_folder(&mut self, folder: &str, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        if repository.explorer.is_expanded(folder) {
            repository.explorer.collapse(folder);
        } else {
            repository.explorer.expand(folder);
            if !repository.explorer.is_read(folder) {
                self.read_folders(vec![folder.to_owned()], None, cx);
            }
        }
        cx.notify();
    }

    fn collapse_all_folders(&mut self, cx: &mut Context<Self>) {
        if let Some(repository) = self.repository.as_mut() {
            repository.explorer.collapse_all();
            cx.notify();
        }
    }

    /// Reads `folders` in the background and shows what they hold; then
    /// selects and scrolls to `reveal`, when given.
    pub(super) fn read_folders(
        &mut self,
        folders: Vec<String>,
        reveal: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let root = repository.root.clone();
        let read = cx.background_spawn({
            let root = root.clone();
            async move {
                folders
                    .into_iter()
                    .map(|folder| {
                        let entries = explorer::read_dir(&root, &folder);
                        (folder, entries)
                    })
                    .collect::<Vec<(String, Vec<DirEntry>)>>()
            }
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let shown = this.update(cx, |this, cx| {
                // Another repository may have opened meanwhile.
                let Some(repository) = this.repository.as_mut().filter(|r| r.root == root) else {
                    return;
                };
                for (folder, entries) in read {
                    repository.explorer.set_children(&folder, entries);
                }
                if let Some(path) = reveal {
                    repository.select_in_explorer(&path);
                }
                cx.notify();
            });
            if let Err(error) = shown {
                log::warn!("view released before a folder was read: {error:#}");
            }
        })
        .detach();
    }

    /// VS Code's `explorer.autoReveal`: shows the active tab's file in the
    /// tree, expanding the folders above it, but not inside ignored folders,
    /// whose files are selected only when already in view.
    pub(super) fn reveal_active_file(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        // Files outside the repository are not in the tree.
        let path = match self.editors.group.active() {
            Some(TabKey::Diff { path, .. }) => path.to_string(),
            Some(TabKey::File(path)) => match repository.relative(path) {
                Some(path) => path,
                None => return,
            },
            Some(TabKey::Settings) | None => return,
        };
        let ignored = repository
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_ignored(&path));
        if ignored {
            if repository.explorer.row(&path).is_some() {
                repository.select_in_explorer(&path);
            }
            return;
        }
        let unread: Vec<String> = explorer::ancestors(&path)
            .filter(|folder| !repository.explorer.is_read(folder))
            .map(str::to_owned)
            .collect();
        for folder in explorer::ancestors(&path) {
            repository.explorer.expand(folder);
        }
        if unread.is_empty() {
            repository.select_in_explorer(&path);
        } else {
            self.read_folders(unread, Some(path), cx);
        }
    }

    /// The tree's keys, as in VS Code's lists.
    fn explorer_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        let rows = repository.explorer.rows();
        let selected = repository
            .explorer
            .selected
            .as_deref()
            .and_then(|path| repository.explorer.row(path))
            .map(|(ix, row)| (ix, row.clone()));
        let keystroke = &event.keystroke;
        let select = |ix: usize| rows.get(ix).map(|row| row.path.clone());
        let target = match (keystroke.key.as_str(), keystroke.modifiers.platform, &selected) {
            ("down", false, None) => select(0),
            ("down", false, Some((ix, _))) => select(ix + 1),
            ("up", false, Some((ix, _))) => ix.checked_sub(1).and_then(select),
            ("right", false, Some((_, row))) if row.is_dir && !row.expanded => {
                let folder = row.path.clone();
                self.toggle_folder(&folder, cx);
                cx.stop_propagation();
                return;
            }
            ("right", false, Some((ix, row))) if row.is_dir => select(ix + 1)
                .filter(|_| rows.get(ix + 1).is_some_and(|next| next.depth > row.depth)),
            ("left", false, Some((_, row))) if row.is_dir && row.expanded => {
                let folder = row.path.clone();
                self.toggle_folder(&folder, cx);
                cx.stop_propagation();
                return;
            }
            ("left", false, Some((_, row))) => {
                row.path.rsplit_once('/').map(|(parent, _)| parent.to_owned())
            }
            ("space", false, Some((_, row))) | ("down", true, Some((_, row))) if !row.is_dir => {
                let preview = keystroke.key == "space";
                let path = row.path.clone();
                if let Some(key) = self.file_key(&path) {
                    self.open_tab(key, preview, window, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        cx.stop_propagation();
        if let Some(path) = target {
            repository.select_in_explorer(&path);
            cx.notify();
        }
    }
}

impl Repository {
    /// Selects `path`'s row and scrolls it into view.
    pub(super) fn select_in_explorer(&mut self, path: &str) {
        self.explorer.selected = Some(path.to_owned());
        if let Some((ix, _)) = self.explorer.row(path) {
            self.explorer_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
    }
}
