//! The editor area: one tab for the open file, its path as breadcrumbs, and
//! the diff of the group it was opened from.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffHunkSeparator, DiffMode},
    h_flex, v_flex,
};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};

use super::{LoadState, Repository, Workbench};
use crate::{file_icons, git::ChangeGroup, theme};

impl Workbench {
    pub(super) fn render_editor(
        &self,
        repository: &Repository,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (danger, muted, background) = {
            let theme = cx.theme();
            (theme.danger, theme.muted_foreground, theme.background)
        };
        let open = repository.open_file();
        let message = match &repository.load_state {
            LoadState::Failed(message) => Some((message.clone(), danger)),
            LoadState::Loading if repository.snapshot.is_none() => {
                Some(("正在读取改动…".into(), muted))
            }
            _ if repository.files().is_empty() => Some(("没有未提交的改动".into(), muted)),
            _ if open.is_none() => Some(("在左侧选择一个文件查看改动".into(), muted)),
            _ => None,
        };
        let header = open.map(|changed| {
            (
                self.render_tab_bar(repository, cx),
                render_breadcrumbs(changed.file.path(), cx),
            )
        });

        v_flex()
            .size_full()
            .bg(background)
            .when_some(header, |this, (tab_bar, breadcrumbs)| {
                this.child(tab_bar).child(breadcrumbs)
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    // The diff stays mounted even while there is nothing to
                    // show: it measures its viewport while painting and asks
                    // for a relayout from there, which GPUI drops mid-draw.
                    // Mounting it only once a file opens can leave it blank
                    // until some unrelated event redraws the window.
                    .child(
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
                    .when_some(message, |this, (message, color)| {
                        this.child(
                            v_flex()
                                .id("diff-message")
                                .test_support()
                                .aria_label(message.clone())
                                .absolute()
                                .inset_0()
                                .items_center()
                                .justify_center()
                                .bg(background)
                                .text_color(color)
                                .child(message),
                        )
                    }),
            )
    }

    fn render_tab_bar(&self, repository: &Repository, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(changed) = repository.open_file() else {
            return div().into_any_element();
        };
        let name = changed
            .file
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let side = match changed.group {
            ChangeGroup::Staged => "索引",
            ChangeGroup::Changes | ChangeGroup::Merge => "工作树",
        };
        let title: SharedString = format!("{name} ({side})").into();
        let split = repository.diff.read(cx).mode() == DiffMode::Split;

        h_flex()
            .h(px(35.))
            .flex_none()
            .bg(theme.tab_bar)
            .child(
                h_flex()
                    .id("editor-tab")
                    .test_support()
                    .aria_label(title.clone())
                    .h_full()
                    .pl(px(10.))
                    .pr(px(6.))
                    .gap(px(6.))
                    .bg(theme.tab_active)
                    .text_color(theme.tab_active_foreground)
                    .border_r_1()
                    .border_color(theme::colors::tab_border())
                    .child(img(file_icons::icon_for(changed.file.path())).size(px(16.)))
                    .child(title)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(changed.letter.color())
                            .child(changed.letter.as_str()),
                    )
                    .child(
                        Button::new("close-editor")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::X))
                            .tooltip("关闭")
                            .on_click(cx.listener(|this, _, _, cx| this.close_editor(cx))),
                    ),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .pr(px(8.))
                    .gap(px(2.))
                    .child(
                        Button::new("previous-change")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowUp)
                            .tooltip("上一处更改")
                            .on_click(cx.listener(|this, _, _, cx| this.move_to_change(false, cx))),
                    )
                    .child(
                        Button::new("next-change")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowDown)
                            .tooltip("下一处更改")
                            .on_click(cx.listener(|this, _, _, cx| this.move_to_change(true, cx))),
                    )
                    .child(
                        Button::new("toggle-inline")
                            .ghost()
                            .small()
                            .icon(Icon::new(if split {
                                Lucide::Rows2
                            } else {
                                Lucide::Columns2
                            }))
                            .tooltip(if split {
                                "切换为内联视图"
                            } else {
                                "切换为并排视图"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_inline(cx))),
                    ),
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
