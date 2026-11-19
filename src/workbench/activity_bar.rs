//! VS Code's activity bar: one icon per side bar view, the active one marked,
//! and the number of changes on Source Control's. Where it goes follows the
//! `workbench.activityBar.location` setting, as in VS Code 1.117: upright
//! left of the side bar, across the side bar's title row, across a row at
//! its bottom, or nowhere.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu},
    v_flex,
};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};

use super::{Repository, SideView, Workbench};
use crate::{
    OpenSettings, ShowExplorer, ShowSourceControl,
    settings::{ActivityBarLocation, SettingsStore},
    theme::colors,
};

/// `.monaco-workbench .activitybar` is 48px wide, its items 48px tall.
const WIDTH: f32 = 48.;
/// Codicons in the activity bar are 24px.
const ICON_SIZE: f32 = 24.;
/// `.part > .title` and `.header-or-footer` are 35px tall.
const ROW_HEIGHT: f32 = 35.;
/// Across the side bar: `.action-item.icon` has `padding: 0 3px`, its
/// `.action-label` 2px around a 16px codicon.
const ROW_ITEM_WIDTH: f32 = 3. + 2. + 16. + 2. + 3.;
const ROW_ICON_SIZE: f32 = 16.;
/// `.part > .title`'s padding before the label, plus `.title-label`'s.
const TITLE_INSET_LEFT: f32 = 20.;
/// `.header-or-footer`'s padding.
const ROW_INSET: f32 = 4.;

/// How an activity item is laid out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Orientation {
    Upright,
    Across,
}

impl Workbench {
    fn activity_bar_location(cx: &App) -> ActivityBarLocation {
        SettingsStore::global(cx).values().activity_bar_location
    }

    /// The upright activity bar left of the side bar, with the 「管理」 gear
    /// at its bottom; `None` for the other locations.
    pub(super) fn render_activity_bar(
        &self,
        repository: &Repository,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if Self::activity_bar_location(cx) != ActivityBarLocation::Default {
            return None;
        }
        Some(
            v_flex()
                .w(px(WIDTH))
                .h_full()
                .flex_none()
                .bg(colors::activity_bar())
                .children(self.activity_items(repository, Orientation::Upright, cx))
                .child(div().flex_1())
                .child(manage_button())
                .into_any_element(),
        )
    }

    /// A side bar view's title row, VS Code's `.part > .title`: the view's
    /// name and its actions, or with the activity bar on top, its icons in
    /// the name's place.
    pub(super) fn side_bar_title(
        &self,
        repository: &Repository,
        label: &'static str,
        actions: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let top = Self::activity_bar_location(cx) == ActivityBarLocation::Top;
        let foreground = cx.theme().foreground;
        h_flex()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .when(top, |this| {
                this.pl(px(ROW_INSET))
                    .child(
                        h_flex()
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .children(self.activity_items(repository, Orientation::Across, cx)),
                    )
            })
            .when(!top, |this| {
                // A narrow side bar cuts the title short, never its actions.
                this.pl(px(TITLE_INSET_LEFT)).child(
                    div()
                        .id(SharedString::from(format!("side-bar-title-{label}")))
                        .test_support()
                        .aria_label(label)
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(foreground)
                        .child(label),
                )
            })
            .children(actions)
            .into_any_element()
    }

    /// The row of icons at the bottom of the side bar when the activity bar
    /// is there, VS Code's `.header-or-footer.footer`.
    pub(super) fn render_activity_footer(
        &self,
        repository: &Repository,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if Self::activity_bar_location(cx) != ActivityBarLocation::Bottom {
            return None;
        }
        Some(
            h_flex()
                .h(px(ROW_HEIGHT))
                .flex_none()
                .px(px(ROW_INSET))
                .children(self.activity_items(repository, Orientation::Across, cx))
                .into_any_element(),
        )
    }

    fn activity_items(
        &self,
        repository: &Repository,
        orientation: Orientation,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let changes = repository.files().len();
        vec![
            self.activity_item(repository, SideView::Explorer, None, orientation, cx),
            self.activity_item(
                repository,
                SideView::SourceControl,
                (changes > 0).then_some(changes),
                orientation,
                cx,
            ),
        ]
    }

    fn activity_item(
        &self,
        repository: &Repository,
        view: SideView,
        badge: Option<usize>,
        orientation: Orientation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = repository.side_view == view;
        let (id, icon, label, tooltip) = match view {
            SideView::Explorer => (
                "activity-explorer",
                Lucide::Files,
                "资源管理器",
                "资源管理器 (⇧⌘E)",
            ),
            SideView::SourceControl => (
                "activity-scm",
                Lucide::GitBranch,
                "源代码管理",
                "源代码管理 (⌃⇧G)",
            ),
        };
        let upright = orientation == Orientation::Upright;
        let (foreground, inactive) = if upright {
            (colors::activity_bar_foreground(), colors::activity_bar_inactive())
        } else {
            (
                colors::activity_bar_top_foreground(),
                colors::activity_bar_top_inactive(),
            )
        };
        let footer = Self::activity_bar_location(cx) == ActivityBarLocation::Bottom;
        // The active view's marker: a 2px line on the left edge upright; a
        // 1px line 2px from the side bar's inside edge across it.
        let marker = div().absolute().bg(foreground);
        let marker = match (upright, footer) {
            (true, _) => marker.left_0().top_0().bottom_0().w(px(2.)),
            (false, false) => marker.left(px(2.)).right(px(2.)).bottom(px(2.)).h(px(1.)),
            (false, true) => marker.left(px(2.)).right(px(2.)).top(px(2.)).h(px(1.)),
        };
        let (width, height, icon_size) = if upright {
            (WIDTH, WIDTH, ICON_SIZE)
        } else {
            (ROW_ITEM_WIDTH, ROW_HEIGHT, ROW_ICON_SIZE)
        };
        div()
            .id(id)
            .test_support()
            .aria_label(label)
            .aria_selected(active)
            .relative()
            .w(px(width))
            .h(px(height))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(if active { foreground } else { inactive })
            .hover(move |style| style.text_color(foreground))
            .when(active, |this| this.child(marker))
            .child(Icon::new(icon).size(px(icon_size)))
            .when_some(badge, |this, count| {
                let text: SharedString = count.to_string().into();
                let badge = div()
                    .id(SharedString::from(format!("{id}-badge")))
                    .test_support()
                    .aria_label(text.clone())
                    .absolute()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(colors::activity_badge())
                    .text_color(colors::activity_bar_foreground());
                // Across the side bar, VS Code's compact badge.
                let badge = if upright {
                    badge
                        .top(px(24.))
                        .right(px(8.))
                        .h(px(16.))
                        .min_w(px(16.))
                        .px(px(4.))
                        .text_size(px(9.))
                } else {
                    badge
                        .top(px(17.))
                        .right_0()
                        .h(px(13.))
                        .min_w(px(13.))
                        .px(px(3.))
                        .text_size(px(9.))
                        .font_weight(FontWeight::SEMIBOLD)
                };
                this.child(badge.child(text))
            })
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| this.show_side_view(view, cx)))
            .into_any_element()
    }

    /// ⇧⌘E: the explorer, with the keyboard in its tree.
    pub(super) fn show_explorer(
        &mut self,
        _: &ShowExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.repository.is_none() {
            return;
        }
        self.show_side_view(SideView::Explorer, cx);
        window.focus(&self.explorer_focus, cx);
    }

    /// ⌃⇧G: source control, with the keyboard in the commit message.
    pub(super) fn show_source_control(
        &mut self,
        _: &ShowSourceControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self
            .repository
            .as_ref()
            .map(|repository| repository.message.clone())
        else {
            return;
        };
        self.show_side_view(SideView::SourceControl, cx);
        message.update(cx, |message, cx| message.focus(window, cx));
    }
}

/// The 「管理」 gear at the bottom of the upright activity bar. VS Code's
/// menu holds many entries; Whit has one.
fn manage_button() -> AnyElement {
    Button::new("activity-manage")
        .ghost()
        .size(px(WIDTH))
        .p_0()
        .text_color(colors::activity_bar_inactive())
        .tooltip("管理")
        .child(Icon::new(Lucide::Settings).size(px(ICON_SIZE)))
        .dropdown_menu_with_anchor(Anchor::BottomLeft, |menu: PopupMenu, _, _| {
            menu.menu("设置", Box::new(OpenSettings))
        })
        .into_any_element()
}
