//! VS Code's Quick Input: the picker and the input box that drop from the top
//! of the window. Callers await the answer, so a flow of several questions
//! reads top to bottom, as VS Code's `await showQuickPick(...)` does.
use std::ops::Range;

use futures_channel::oneshot;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _, WindowExt as _,
    command::{Command, CommandItem, CommandState},
    h_flex,
    input::{
        Escape, Input, InputEvent, InputState, MovePageDown, MovePageUp, MoveToEnd, MoveToStart,
    },
    notification::Notification,
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use super::Workbench;
use crate::theme::colors;

/// VS Code's quick input width: 62% of the window, at most 600 pixels.
const MAX_WIDTH: f32 = 600.;
const WIDTH_RATIO: f32 = 0.62;
const LIST_MAX_HEIGHT: f32 = 440.;
/// How far PageUp and PageDown move the highlight.
const PAGE_ITEMS: usize = 10;

/// One row of a picker.
#[derive(Clone)]
pub struct PickItem {
    pub icon: Option<Icon>,
    pub label: SharedString,
    /// Muted text after the label.
    pub description: Option<SharedString>,
    /// Muted second line.
    pub detail: Option<SharedString>,
}

impl PickItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            label: label.into(),
            description: None,
            detail: None,
        }
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// A titled run of items; the title shows at the right of its first row.
pub struct PickGroup<T> {
    pub label: Option<SharedString>,
    pub items: Vec<(PickItem, T)>,
}

/// What a picker offers: `actions` are always shown, whatever was typed, and
/// come first until something is typed, then after the matching groups.
pub struct PickList<T> {
    pub actions: Vec<(PickItem, T)>,
    pub groups: Vec<PickGroup<T>>,
}

impl<T> Default for PickList<T> {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            groups: Vec::new(),
        }
    }
}

impl<T> PickList<T> {
    /// Separates what is drawn from the values handed back, numbering items
    /// actions first, then each group in order.
    fn split(self) -> (Entries, Vec<T>) {
        let mut values = Vec::new();
        let mut take = |items: Vec<(PickItem, T)>| -> Vec<PickItem> {
            items
                .into_iter()
                .map(|(item, value)| {
                    values.push(value);
                    item
                })
                .collect()
        };
        let actions = take(self.actions);
        let groups = self
            .groups
            .into_iter()
            .map(|group| (group.label, take(group.items)))
            .collect();
        (Entries { actions, groups }, values)
    }
}

struct Entries {
    actions: Vec<PickItem>,
    groups: Vec<(Option<SharedString>, Vec<PickItem>)>,
}

/// The confirmed item and the text typed when it was confirmed.
pub struct Picked<T> {
    pub value: T,
    pub query: String,
}

enum Row<'a> {
    Separator,
    Item {
        id: usize,
        item: &'a PickItem,
        group: Option<SharedString>,
    },
}

pub struct QuickPick {
    state: Entity<CommandState>,
    placeholder: SharedString,
    /// `None` while the items are still being read.
    entries: Option<Entries>,
    query: SharedString,
    /// The id of each item row as last rendered, which is how the command
    /// list's row index maps back to an item.
    shown_ids: Vec<usize>,
    done: Option<oneshot::Sender<(usize, String)>>,
}

impl EventEmitter<DismissEvent> for QuickPick {}

impl QuickPick {
    fn new(
        placeholder: SharedString,
        done: oneshot::Sender<(usize, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| {
            let mut state = CommandState::new(window, cx);
            state.set_loading(true, window, cx);
            state
        });
        state.update(cx, |state, cx| state.focus(window, cx));
        Self {
            state,
            placeholder,
            entries: None,
            query: SharedString::default(),
            shown_ids: Vec::new(),
            done: Some(done),
        }
    }

    fn set_entries(&mut self, entries: Entries, window: &mut Window, cx: &mut Context<Self>) {
        self.entries = Some(entries);
        self.state
            .update(cx, |state, cx| state.set_loading(false, window, cx));
        cx.notify();
    }

    /// The rows for the current query, in VS Code's order: with nothing typed,
    /// the actions and then every group; otherwise the matching items of each
    /// group, then the actions after a separator.
    fn rows(&self) -> Vec<Row<'_>> {
        let Some(entries) = self.entries.as_ref() else {
            return Vec::new();
        };
        let query = self.query.trim();
        let action_rows = entries
            .actions
            .iter()
            .enumerate()
            .map(|(id, item)| Row::Item {
                id,
                item,
                group: None,
            });
        let mut group_rows = Vec::new();
        let mut id = entries.actions.len();
        for (label, items) in &entries.groups {
            let mut first = true;
            for item in items {
                if query.is_empty() || find_ignoring_case(&item.label, query).is_some() {
                    if first && !group_rows.is_empty() {
                        group_rows.push(Row::Separator);
                    }
                    group_rows.push(Row::Item {
                        id,
                        item,
                        group: if first { label.clone() } else { None },
                    });
                    first = false;
                }
                id += 1;
            }
        }
        let mut rows: Vec<Row> = Vec::new();
        if query.is_empty() {
            rows.extend(action_rows);
            if !rows.is_empty() && !group_rows.is_empty() {
                rows.push(Row::Separator);
            }
            rows.extend(group_rows);
        } else {
            rows.extend(group_rows);
            if !rows.is_empty() && !entries.actions.is_empty() {
                rows.push(Row::Separator);
            }
            rows.extend(action_rows);
        }
        rows
    }

    fn confirm(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(&id) = self.shown_ids.get(row) else {
            return;
        };
        if let Some(done) = self.done.take()
            && done.send((id, self.query.to_string())).is_err()
        {
            log::warn!("nobody awaited the picked item");
        }
        cx.emit(DismissEvent);
    }

    /// Moves the highlight to the `row`-th item, clamped to the list.
    fn highlight(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.shown_ids.len().checked_sub(1) else {
            return;
        };
        let path = IndexPath::new(row.min(last)).section(0);
        self.state.update(cx, |state, cx| {
            state.set_selected_index(Some(path), window, cx)
        });
    }

    fn highlighted_row(&self, cx: &App) -> usize {
        self.state
            .read(cx)
            .selected_index()
            .map_or(0, |path| path.row)
    }

    /// The labels of the rows as shown, separators as `—`, a group title as
    /// `label [title]`.
    #[cfg(test)]
    pub(super) fn shown_labels(&self) -> Vec<String> {
        self.rows()
            .iter()
            .map(|row| match row {
                Row::Separator => "—".to_owned(),
                Row::Item {
                    item,
                    group: Some(group),
                    ..
                } => format!("{} [{group}]", item.label),
                Row::Item { item, .. } => item.label.to_string(),
            })
            .collect()
    }

    /// Confirms the item labelled `label`, as clicking it would.
    #[cfg(test)]
    pub(super) fn choose(&mut self, label: &str, cx: &mut Context<Self>) {
        let row = self
            .rows()
            .iter()
            .filter(|row| matches!(row, Row::Item { .. }))
            .position(|row| matches!(row, Row::Item { item, .. } if item.label == label))
            .unwrap_or_else(|| panic!("no item {label:?} in {:?}", self.shown_labels()));
        self.confirm(row, cx);
    }

    #[cfg(test)]
    pub(super) fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    #[cfg(test)]
    pub(super) fn is_loading(&self, cx: &App) -> bool {
        self.state.read(cx).is_loading()
    }
}

impl Render for QuickPick {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        let query = self.query.trim().to_owned();
        let weak = cx.entity().downgrade();
        let mut command = Command::new(&self.state)
            .placeholder(self.placeholder.clone())
            .filterable(false)
            .bordered(false)
            .search_icon(false)
            .max_h(px(LIST_MAX_HEIGHT))
            .empty(|_, _, cx| {
                div()
                    .px(px(11.))
                    .py(px(4.))
                    .text_color(cx.theme().muted_foreground)
                    .child("没有匹配的结果")
            })
            .on_query({
                let weak = weak.clone();
                move |query, _, cx| {
                    let updated = weak.update(cx, |this, cx| {
                        this.query = SharedString::from(query.to_owned());
                        cx.notify();
                    });
                    if let Err(error) = updated {
                        log::warn!("picker released while typing: {error:#}");
                    }
                }
            })
            .on_confirm(move |path, _, cx| {
                if let Err(error) = weak.update(cx, |this, cx| this.confirm(path.row, cx)) {
                    log::warn!("picker released while confirming: {error:#}");
                }
            });
        let mut shown_ids = Vec::new();
        for row in rows {
            command = match row {
                Row::Separator => command.separator(),
                Row::Item { id, item, group } => {
                    shown_ids.push(id);
                    let highlight = find_ignoring_case(&item.label, &query);
                    let item = item.clone();
                    let label = item.label.clone();
                    command.item(CommandItem::new().label(label).child(move |_, cx| {
                        render_row(&item, highlight.clone(), group.clone(), cx)
                    }))
                }
            };
        }
        self.shown_ids = shown_ids;

        let width = (window.viewport_size().width * WIDTH_RATIO).min(px(MAX_WIDTH));
        frame(width, cx)
            .id("quick-pick")
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.stop_propagation();
                cx.emit(DismissEvent);
            }))
            .capture_action(cx.listener(|this, _: &MoveToStart, window, cx| {
                cx.stop_propagation();
                this.highlight(0, window, cx);
            }))
            .capture_action(cx.listener(|this, _: &MoveToEnd, window, cx| {
                cx.stop_propagation();
                this.highlight(usize::MAX, window, cx);
            }))
            .capture_action(cx.listener(|this, _: &MovePageUp, window, cx| {
                cx.stop_propagation();
                let row = this.highlighted_row(cx).saturating_sub(PAGE_ITEMS);
                this.highlight(row, window, cx);
            }))
            .capture_action(cx.listener(|this, _: &MovePageDown, window, cx| {
                cx.stop_propagation();
                let row = this.highlighted_row(cx).saturating_add(PAGE_ITEMS);
                this.highlight(row, window, cx);
            }))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .child(command)
    }
}

/// The floating box both kinds of quick input share.
fn frame(width: Pixels, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .occlude()
        .w(width)
        .p(px(6.))
        .gap(px(6.))
        .bg(colors::quick_input_background())
        .text_color(theme.foreground)
        .rounded(px(6.))
        // `.quick-input-widget { box-shadow: 0 0 8px 2px var(--vscode-widget-shadow) }`
        .shadow(vec![BoxShadow {
            color: colors::widget_shadow(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(8.),
            spread_radius: px(2.),
            inset: false,
        }])
}

fn render_row(
    item: &PickItem,
    highlight: Option<Range<usize>>,
    group: Option<SharedString>,
    cx: &App,
) -> Div {
    let muted = cx.theme().muted_foreground;
    let label = StyledText::new(item.label.clone()).with_highlights(highlight.map(|range| {
        (
            range,
            HighlightStyle {
                color: Some(colors::list_highlight()),
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
        )
    }));
    v_flex()
        .flex_1()
        .min_w_0()
        .text_size(px(13.))
        .child(
            h_flex()
                .gap(px(6.))
                .min_w_0()
                .when_some(item.icon.clone(), |this, icon| {
                    this.child(icon.size(px(16.)).flex_none())
                })
                .child(div().flex_none().child(label))
                .when_some(item.description.clone(), |this, description| {
                    this.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(description),
                    )
                })
                .child(div().flex_1())
                .when_some(group, |this, group| {
                    this.child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(group),
                    )
                }),
        )
        .when_some(item.detail.clone(), |this, detail| {
            this.child(
                div()
                    .min_w_0()
                    .truncate()
                    .when(item.icon.is_some(), |this| this.pl(px(22.)))
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(detail),
            )
        })
}

/// Where `needle` occurs in `haystack`, ignoring case, as a byte range of
/// `haystack`.
fn find_ignoring_case(haystack: &str, needle: &str) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    haystack.char_indices().find_map(|(start, _)| {
        let mut rest = haystack[start..].chars();
        let mut end = start;
        for wanted in needle.chars() {
            let got = rest.next()?;
            if !got.to_lowercase().eq(wanted.to_lowercase()) {
                return None;
            }
            end += got.len_utf8();
        }
        Some(start..end)
    })
}

/// How a typed value fares, checked after every keystroke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Validation {
    Valid,
    /// Shown, and Enter still accepts.
    Info(SharedString),
    /// Shown, and Enter does nothing.
    Error(SharedString),
}

pub struct InputOptions {
    pub placeholder: SharedString,
    pub prompt: SharedString,
    pub value: String,
    pub validate: Box<dyn Fn(&str) -> Validation>,
}

pub struct InputBox {
    input: Entity<InputState>,
    prompt: SharedString,
    validate: Box<dyn Fn(&str) -> Validation>,
    validation: Validation,
    done: Option<oneshot::Sender<String>>,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for InputBox {}

impl InputBox {
    fn new(
        options: InputOptions,
        done: oneshot::Sender<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let validation = (options.validate)(&options.value);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(options.placeholder)
                .default_value(options.value)
        });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        let subscription = cx.subscribe_in(&input, window, |this, input, event, _, cx| {
            let value = input.read(cx).value();
            match event {
                InputEvent::Change => {
                    this.validation = (this.validate)(&value);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.accept(value.to_string(), cx),
                _ => {}
            }
        });
        Self {
            input,
            prompt: options.prompt,
            validate: options.validate,
            validation,
            done: Some(done),
            _subscription: subscription,
        }
    }

    fn accept(&mut self, value: String, cx: &mut Context<Self>) {
        if matches!(self.validation, Validation::Error(_)) {
            return;
        }
        if let Some(done) = self.done.take()
            && done.send(value).is_err()
        {
            log::warn!("nobody awaited the typed value");
        }
        cx.emit(DismissEvent);
    }

    #[cfg(test)]
    pub(super) fn validation(&self) -> &Validation {
        &self.validation
    }

    /// Puts the caret back in the field after a test clicked elsewhere.
    #[cfg(test)]
    pub(super) fn focus_for_test(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }
}

impl Render for InputBox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = (window.viewport_size().width * WIDTH_RATIO).min(px(MAX_WIDTH));
        let muted = cx.theme().muted_foreground;
        let message = match &self.validation {
            Validation::Valid => {
                div()
                    .px(px(4.))
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(format!(
                        "{}(按 \"Enter\" 以确认或按 \"Esc\" 以取消)",
                        self.prompt
                    ))
            }
            // `inputValidation.info*` and `inputValidation.error*`, VS Code's dark defaults.
            Validation::Info(text) => validation_box(text.clone(), rgb(0x063B49), rgb(0x007ACC)),
            Validation::Error(text) => validation_box(text.clone(), rgb(0x5A1D1D), rgb(0xBE1100)),
        };
        frame(width, cx)
            .id("quick-input-box")
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.stop_propagation();
                cx.emit(DismissEvent);
            }))
            .child(Input::new(&self.input).small())
            .child(message)
    }
}

fn validation_box(text: SharedString, background: Rgba, border: Rgba) -> Div {
    div()
        .px(px(6.))
        .py(px(4.))
        .text_size(px(12.))
        .bg(background)
        .border_1()
        .border_color(border)
        .child(text)
}

/// The quick input on screen, if any.
pub(super) enum QuickInput {
    Pick(Entity<QuickPick>),
    Input(Entity<InputBox>),
}

impl QuickInput {
    pub(super) fn view(&self) -> AnyView {
        match self {
            QuickInput::Pick(pick) => pick.clone().into(),
            QuickInput::Input(input) => input.clone().into(),
        }
    }
}

impl Workbench {
    fn show_quick_input(
        &mut self,
        quick_input: QuickInput,
        subscription: Subscription,
        cx: &mut Context<Self>,
    ) {
        // Replacing an open one drops its sender, so whoever awaited it gets
        // no answer.
        self.quick_input = Some((quick_input, subscription));
        cx.notify();
    }

    /// Closes the quick input and gives focus back to the window.
    pub(super) fn dismiss_quick_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_input.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// Dismisses a picker when the window loses focus, as VS Code does; an
    /// input box stays, so a name being typed survives a look elsewhere.
    pub(super) fn dismiss_picker_on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.quick_input, Some((QuickInput::Pick(_), _))) {
            self.dismiss_quick_input(window, cx);
        }
    }

    fn open_pick(
        &mut self,
        placeholder: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<QuickPick>, oneshot::Receiver<(usize, String)>) {
        let (sender, receiver) = oneshot::channel();
        let pick = cx.new(|cx| QuickPick::new(placeholder, sender, window, cx));
        let subscription =
            cx.subscribe_in(&pick, window, |this, _, _: &DismissEvent, window, cx| {
                this.dismiss_quick_input(window, cx)
            });
        self.show_quick_input(QuickInput::Pick(pick.clone()), subscription, cx);
        (pick, receiver)
    }

    fn open_input(
        &mut self,
        options: InputOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<String> {
        let (sender, receiver) = oneshot::channel();
        let input = cx.new(|cx| InputBox::new(options, sender, window, cx));
        let subscription =
            cx.subscribe_in(&input, window, |this, _, _: &DismissEvent, window, cx| {
                this.dismiss_quick_input(window, cx)
            });
        self.show_quick_input(QuickInput::Input(input), subscription, cx);
        receiver
    }
}

/// Shows a picker at once, fills it when `list` arrives, and returns the
/// confirmed value, or `None` when dismissed. A failure to read the list
/// closes the picker and reports it.
pub(super) async fn pick<T>(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    placeholder: impl Into<SharedString>,
    list: impl Future<Output = Result<PickList<T>, crate::git::GitError>>,
) -> Option<Picked<T>> {
    let placeholder = placeholder.into();
    // Only the window holds the picker strongly: once it is dismissed or
    // replaced, it and its sender go, and `receiver` reports no answer.
    let (pick, receiver) = this
        .update_in(cx, |this, window, cx| {
            let (pick, receiver) = this.open_pick(placeholder, window, cx);
            (pick.downgrade(), receiver)
        })
        .ok()?;
    let mut values = match list.await {
        Ok(list) => {
            let (entries, values) = list.split();
            pick.update_in(cx, |pick, window, cx| pick.set_entries(entries, window, cx))
                .ok()?;
            values
        }
        Err(error) => {
            let reported = this.update_in(cx, |this, window, cx| {
                this.dismiss_quick_input(window, cx);
                window.push_notification(
                    Notification::error(error.to_string())
                        .title("读取引用失败")
                        .autohide(false),
                    cx,
                );
            });
            if let Err(error) = reported {
                log::warn!("view released before reporting a picker failure: {error:#}");
            }
            return None;
        }
    };
    let (id, query) = receiver.await.ok()?;
    (id < values.len()).then(|| Picked {
        value: values.swap_remove(id),
        query,
    })
}

/// Shows an input box and returns the accepted text, or `None` on Escape.
pub(super) async fn input(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    options: InputOptions,
) -> Option<String> {
    let receiver = this
        .update_in(cx, |this, window, cx| this.open_input(options, window, cx))
        .ok()?;
    receiver.await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn case_insensitive_matching_reports_byte_ranges() {
        assert_eq!(find_ignoring_case("main-next", "MAIN"), Some(0..4));
        assert_eq!(find_ignoring_case("feature/Login", "login"), Some(8..13));
        assert_eq!(find_ignoring_case("中文分支", "分支"), Some(6..12));
        assert_eq!(find_ignoring_case("dev", "main"), None);
        assert_eq!(find_ignoring_case("dev", ""), None);
    }
}
