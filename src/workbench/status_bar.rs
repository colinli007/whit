//! The status bar: the branch, and the sync entry with its incoming and
//! outgoing commit counts.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::{
    Animation, TestSupportExt as _, Transformation, percentage, prelude::FluentBuilder as _, *,
};
use std::time::Duration;

use super::{Repository, Workbench};
use crate::git::Head;

impl Workbench {
    pub(super) fn render_status_bar(
        &self,
        repository: &Repository,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let foreground = theme.foreground;
        let hover = theme.list_hover;
        let syncing = repository.is_syncing();
        let busy = repository.is_busy();

        let item = |id: &'static str| {
            h_flex()
                .id(id)
                .test_support()
                .h_full()
                .px(px(6.))
                .gap(px(4.))
                .hover(move |style| style.bg(hover))
        };

        let branch = repository.branch().map(|branch| {
            let name = match &branch.head {
                Head::Branch(name) | Head::Detached(name) => name.clone(),
            };
            let label: SharedString =
                format!("{name}{}", if branch.dirty { "*" } else { "" }).into();
            item("status-branch")
                .aria_label(label.clone())
                .child(Icon::new(Lucide::GitBranch).size(px(14.)))
                .child(label)
                .when(!busy, |this| {
                    this.cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| this.checkout(window, cx)))
                })
        });

        // A detached HEAD is no branch: nothing to sync or publish, so VS Code
        // shows no sync entry then (a stopped rebase included).
        let sync = repository
            .branch()
            .filter(|branch| matches!(branch.head, Head::Branch(_)))
            .map(|branch| {
                let (icon, label): (Lucide, SharedString) = match &branch.upstream {
                    Some(upstream) => (
                        Lucide::RefreshCw,
                        format!("{}↓ {}↑", upstream.behind, upstream.ahead).into(),
                    ),
                    None => (Lucide::CloudUpload, "发布分支".into()),
                };
                // In step with its upstream, VS Code shows the icon alone; the
                // counts stay in the accessible label.
                let in_step = branch
                    .upstream
                    .as_ref()
                    .is_some_and(|upstream| upstream.behind == 0 && upstream.ahead == 0);
                let icon = Icon::new(icon).size(px(14.));
                item("status-sync")
                    .aria_label(label.clone())
                    .map(|this| {
                        if syncing {
                            this.child(icon.with_animation(
                                "status-sync-spin",
                                Animation::new(Duration::from_secs(1)).repeat(),
                                |icon, delta| {
                                    icon.transform(Transformation::rotate(percentage(delta)))
                                },
                            ))
                        } else {
                            this.child(icon)
                        }
                    })
                    .when(!in_step, |this| this.child(label))
                    .when(!busy, |this| {
                        this.cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.sync(window, cx)))
                    })
            });

        h_flex()
            .h(px(22.))
            .flex_none()
            .px(px(4.))
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .text_size(px(12.))
            .text_color(foreground)
            .children(branch)
            .children(sync)
    }
}
