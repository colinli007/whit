//! The page shown until a folder is open, after VS Code's welcome page: a
//! "Start" column with Open and a "Recent" column of folders.
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};

use super::Workbench;
use crate::{codicons::Codicon, recent::display_parts};

/// How many recent folders show before "More…".
const RECENT_SHOWN: usize = 10;

impl Workbench {
    pub(super) fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let link = theme.link;
        let muted = theme.muted_foreground;
        let danger = theme.danger;

        let start = v_flex().gap(px(8.)).child(heading("启动")).child(
            h_flex()
                .id("open-folder")
                .test_support()
                .aria_label("打开…")
                .gap(px(6.))
                .text_color(link)
                .cursor_pointer()
                .hover(|style| style.underline())
                .child(Icon::new(Codicon::FolderOpened).size(px(16.)))
                .child("打开…")
                .on_click(cx.listener(|this, _, window, cx| this.prompt_open_folder(window, cx))),
        );

        let folders = self.recent.folders();
        let shown = if self.show_all_recent {
            folders.len()
        } else {
            folders.len().min(RECENT_SHOWN)
        };
        let home = dirs::home_dir();
        let recent = v_flex()
            .gap(px(8.))
            .child(heading("最近"))
            .when(folders.is_empty(), |this| {
                this.child(div().text_color(muted).child("没有最近打开的文件夹"))
            })
            .children(folders[..shown].iter().enumerate().map(|(ix, folder)| {
                let (name, parent) = display_parts(folder, home.as_deref());
                let folder = folder.clone();
                h_flex()
                    .id(("recent-folder", ix))
                    .test_support()
                    .aria_label(name.clone())
                    .w_full()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_none()
                            .text_color(link)
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .child(name),
                    )
                    .child(div().min_w_0().truncate().text_color(muted).child(parent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_recent(folder.clone(), window, cx)
                    }))
            }))
            .when(shown < folders.len(), |this| {
                this.child(
                    div()
                        .id("recent-more")
                        .test_support()
                        .aria_label("更多…")
                        .text_color(link)
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .child("更多…")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_all_recent = true;
                            cx.notify();
                        })),
                )
            });

        div()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .pt(px(140.))
                    .pb(px(40.))
                    .justify_center()
                    .items_start()
                    .gap(px(120.))
                    .child(
                        v_flex()
                            .w(px(360.))
                            .gap(px(32.))
                            .child(
                                v_flex()
                                    .child(div().text_size(px(32.)).child(crate::APP_NAME))
                                    .child(
                                        div()
                                            .text_size(px(18.))
                                            .text_color(muted)
                                            .child("查看改动，提交上传"),
                                    ),
                            )
                            .when_some(self.open_error.clone(), |this, message| {
                                this.child(
                                    div()
                                        .id("open-error")
                                        .test_support()
                                        .aria_label(message.clone())
                                        .text_color(danger)
                                        .child(message),
                                )
                            })
                            .child(start),
                    )
                    .child(div().w(px(420.)).pt(px(68.)).child(recent)),
            )
    }
}

fn heading(text: &'static str) -> impl IntoElement {
    div().text_size(px(19.)).child(text)
}
