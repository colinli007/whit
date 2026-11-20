//! VS Code 1.117's settings editor, opened as an editor tab (specs
//! `settings-editor`): a search box over the 「用户」 page, a table of
//! contents and the settings, each with its title, description and control.
//! It shows what the settings store holds and asks the window to write
//! changes, which may be refused (`Workbench::write_setting`).
//!
//! Sizes are VS Code's `settings-editor` styles. The settings are a static
//! table walked on every render; there are four, so no virtual list.
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Escape, Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};
use serde_json::Value;

use crate::{
    FocusSettingsSearch,
    codicons::Codicon,
    settings::{Category, Kind, SETTINGS, SettingItem, SettingsStore},
    theme::colors,
};

/// The key context the page's own bindings (⌘F) apply in.
pub const SETTINGS_EDITOR_CONTEXT: &str = "SettingsEditor";

/// VS Code's `@modified` filter.
const MODIFIED: &str = "@modified";
/// `SettingsEditor2.TOC_RESET_WIDTH`.
const TOC_WIDTH: f32 = 200.;
/// `SettingsEditor2.NARROW_TOTAL_WIDTH`: narrower, the contents go.
const NARROW_WIDTH: f32 = 700.;
/// `.settings-header` and `.monaco-tl-contents`: centered, at most this wide.
const MAX_WIDTH: f32 = 1400.;
/// `.monaco-tl-contents`' side padding.
const CONTENT_INSET: f32 = 24.;
/// `.setting-toolbar-container`: the gear left of a setting.
const GEAR_SIZE: f32 = 22.;
/// `.setting-item-enum select` is 320px wide, 26px tall.
const SELECT_WIDTH: f32 = 320.;
const SELECT_HEIGHT: f32 = 26.;

pub(crate) enum SettingsEditorEvent {
    /// The user changed a setting; the window writes it.
    Write { id: &'static str, value: Value },
}

pub(crate) struct SettingsEditor {
    focus_handle: FocusHandle,
    pub(super) search: Entity<InputState>,
    /// The search box's text, lowercased.
    query: String,
    /// Per setting, built once: the lowercase words of its ID and title,
    /// and its description, for the search.
    index: Vec<SearchEntry>,
    list_scroll: ScrollHandle,
    /// The list child each category's heading is, in this render.
    headings: Vec<(usize, Category)>,
    /// The category picked in the table of contents, until the list
    /// scrolls: a short list cannot scroll to it.
    picked: Option<Category>,
    hovered: Option<&'static str>,
    /// The row last clicked: VS Code's focused row.
    focused_row: Option<&'static str>,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<SettingsEditorEvent> for SettingsEditor {}

impl Focusable for SettingsEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct SearchEntry {
    words: Vec<String>,
    description: String,
}

impl SearchEntry {
    fn of(item: &SettingItem) -> Self {
        let mut words: Vec<String> = Vec::new();
        for part in item.id.split('.') {
            words.extend(camel_words(part));
        }
        for part in [item.title.0, item.title.1] {
            words.extend(part.split_whitespace().map(str::to_lowercase));
        }
        words.extend(
            item.description
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_lowercase),
        );
        Self {
            words,
            description: item.description.to_lowercase(),
        }
    }

    /// Whether `word` starts one of the words; Chinese has no spaces to
    /// find words by, so it is looked for anywhere in the description.
    fn matches(&self, word: &str) -> bool {
        self.words.iter().any(|own| own.starts_with(word))
            || (!word.is_ascii() && self.description.contains(word))
    }
}

/// `enableSmartCommit` as `enable`, `smart`, `commit`.
fn camel_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_uppercase() && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

impl SettingsEditor {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索设置"));
        let subscriptions = [
            cx.subscribe(&search, |this, search, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = search.read(cx).value().to_lowercase();
                    this.picked = None;
                    cx.notify();
                }
            }),
            cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
        ];
        Self {
            focus_handle: cx.focus_handle(),
            search,
            query: String::new(),
            index: SETTINGS.iter().map(SearchEntry::of).collect(),
            list_scroll: ScrollHandle::new(),
            headings: Vec::new(),
            picked: None,
            hovered: None,
            focused_row: None,
            _subscriptions: subscriptions,
        }
    }

    /// Puts the keyboard in the search box with its text selected.
    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search.update(cx, |search, cx| {
            search.focus(window, cx);
            search.select_all(window, cx);
        });
    }

    /// Replaces the search text. Setting it is not typing, so the search
    /// box tells nothing: the query follows here.
    fn set_query(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.query = text.to_lowercase();
        self.picked = None;
        self.search
            .update(cx, |search, cx| search.set_value(text, window, cx));
        cx.notify();
    }

    /// The settings the search leaves, in table order.
    fn visible(&self, cx: &App) -> Vec<&'static SettingItem> {
        let store = SettingsStore::global(cx);
        let mut modified_only = false;
        let mut words = Vec::new();
        for word in self.query.split_whitespace() {
            if word == MODIFIED {
                modified_only = true;
            } else {
                words.push(word);
            }
        }
        SETTINGS
            .iter()
            .zip(&self.index)
            .filter(|(item, entry)| {
                (!modified_only || store.is_configured(item.id))
                    && words.iter().all(|word| entry.matches(word))
            })
            .map(|(item, _)| item)
            .collect()
    }

    fn write(&mut self, id: &'static str, value: Value, cx: &mut Context<Self>) {
        cx.emit(SettingsEditorEvent::Write { id, value });
    }

    /// The table of contents' highlighted category: the one picked there,
    /// or the first whose heading is at or above the list's top.
    fn current_category(&self) -> Option<Category> {
        if let Some(picked) = self.picked {
            return Some(picked);
        }
        let top = self.list_scroll.top_item();
        self.headings
            .iter()
            .rev()
            .find(|(ix, _)| *ix <= top)
            .or(self.headings.first())
            .map(|(_, category)| *category)
    }

    fn pick(&mut self, category: Category, cx: &mut Context<Self>) {
        if let Some((ix, _)) = self.headings.iter().find(|(_, c)| *c == category) {
            self.list_scroll.scroll_to_top_of_item(*ix);
        }
        self.picked = Some(category);
        cx.notify();
    }

    fn render_header(&self, count: Option<usize>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let modified_only = self.query.split_whitespace().any(|word| word == MODIFIED);
        let editor = cx.entity().downgrade();
        let count_text: Option<SharedString> = count.map(|count| match count {
            0 => "未找到设置".into(),
            1 => "找到 1 个设置".into(),
            n => format!("找到 {n} 个设置").into(),
        });
        let filter = Button::new("settings-filter")
            .ghost()
            .xsmall()
            .icon(Icon::new(Codicon::Filter))
            .tooltip("筛选设置")
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let editor = editor.clone();
                menu.item(
                    PopupMenuItem::new("已更改")
                        .checked(modified_only)
                        .on_click(move |_, window, cx| {
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.toggle_modified(window, cx)
                                });
                            }
                        }),
                )
            });
        let search = Input::new(&self.search)
            .cleanable(true)
            .suffix(
                h_flex()
                    .gap(px(3.))
                    .when_some(count_text, |this, text| {
                        this.child(
                            div()
                                .id("settings-count")
                                .test_support()
                                .aria_label(text.clone())
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(text),
                        )
                    })
                    .child(filter),
            );

        v_flex()
            .w_full()
            .max_w(px(MAX_WIDTH))
            .mx_auto()
            // `.settings-header`: 11px margin, 3px padding above.
            .pt(px(14.))
            .px(px(CONTENT_INSET))
            .child(
                div()
                    .key_context("SettingsSearch")
                    .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                        if !this.query.is_empty() {
                            cx.stop_propagation();
                            this.set_query(String::new(), window, cx);
                        }
                    }))
                    .child(search),
            )
            .child(
                // `.settings-header-controls`: the scope tabs over a line.
                h_flex()
                    .mt(px(10.))
                    .border_b_1()
                    .border_color(colors::settings_header_border())
                    .child(
                        div()
                            .id("settings-target-user")
                            .test_support()
                            .aria_label("用户")
                            .aria_selected(true)
                            .px(px(8.))
                            .pt(px(7.))
                            .pb(px(6.))
                            .text_size(px(13.))
                            .text_color(colors::settings_header_foreground())
                            .border_b_1()
                            .border_color(colors::settings_header_foreground())
                            .child("用户"),
                    ),
            )
    }

    fn toggle_modified(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut words: Vec<&str> = self.query.split_whitespace().collect();
        if words.contains(&MODIFIED) {
            words.retain(|word| *word != MODIFIED);
        } else {
            words.insert(0, MODIFIED);
        }
        let text = words.join(" ");
        self.set_query(text, window, cx);
    }

    fn render_toc(&self, current: Option<Category>, visible: &[&SettingItem], cx: &mut Context<Self>) -> AnyElement {
        let foreground = cx.theme().foreground;
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut parent = None;
        for category in Category::ALL {
            if !visible.iter().any(|item| item.category == category) {
                continue;
            }
            if parent != Some(category.parent()) {
                parent = Some(category.parent());
                rows.push(self.toc_row(category.parent(), 0, false, category, cx));
            }
            rows.push(self.toc_row(category.name(), 1, current == Some(category), category, cx));
        }
        v_flex()
            .id("settings-toc")
            .test_support()
            .w(px(TOC_WIDTH))
            .flex_none()
            .h_full()
            .text_color(foreground)
            .children(rows)
            .into_any_element()
    }

    fn toc_row(
        &self,
        label: &'static str,
        depth: usize,
        selected: bool,
        category: Category,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .id(SharedString::from(format!("settings-toc-{label}")))
            .test_support()
            .aria_label(label)
            .aria_selected(selected)
            .h(px(22.))
            .pl(px(8. + depth as f32 * 16.))
            .gap(px(2.))
            .cursor_pointer()
            .when(!selected, |this| this.opacity(0.9))
            .when(selected, |this| this.font_weight(FontWeight::BOLD))
            .when(depth == 0, |this| {
                this.child(Icon::new(Codicon::ChevronDown).size(px(16.)))
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.pick(category, cx)))
            .into_any_element()
    }

    /// The list's children and the child each heading is.
    fn render_list(
        &self,
        visible: &[&'static SettingItem],
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Vec<(usize, Category)>) {
        let mut children = Vec::new();
        let mut headings = Vec::new();
        let mut parent = None;
        for category in Category::ALL {
            let items: Vec<&'static SettingItem> = visible
                .iter()
                .copied()
                .filter(|item| item.category == category)
                .collect();
            if items.is_empty() {
                continue;
            }
            if parent != Some(category.parent()) {
                parent = Some(category.parent());
                children.push(heading(category.parent(), 26.));
            }
            headings.push((children.len(), category));
            children.push(heading(category.name(), 22.));
            for item in items {
                children.push(self.render_item(item, cx));
            }
        }
        (children, headings)
    }

    fn render_item(&self, item: &'static SettingItem, cx: &mut Context<Self>) -> AnyElement {
        let foreground = cx.theme().foreground;
        let store = SettingsStore::global(cx);
        let value = item.value(store.values());
        let configured = store.is_configured(item.id);
        let hovered = self.hovered == Some(item.id);
        let focused = self.focused_row == Some(item.id);
        let is_bool = matches!(item.kind, Kind::Bool);
        let id = item.id;
        let title = h_flex()
            .text_color(colors::settings_header_foreground())
            .font_weight(FontWeight::SEMIBOLD)
            .pb(px(2.))
            .child(div().opacity(0.9).child(format!("{}: ", item.title.0)))
            .child(item.title.1);
        let control = match item.kind {
            Kind::Bool => {
                let checked = value.as_bool().unwrap_or_default();
                // `.setting-value-checkbox`: 18px, 3px corners, 9px before
                // the description, which toggles it too.
                h_flex()
                    .id(SharedString::from(format!("setting-{id}-control")))
                    .test_support()
                    .aria_label(item.description)
                    .aria_selected(checked)
                    .items_start()
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(18.))
                            .flex_none()
                            .mr(px(9.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(colors::settings_control())
                            .bg(colors::settings_control())
                            .text_color(colors::settings_control_foreground())
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(checked, |this| {
                                this.child(Icon::new(Codicon::Check).size(px(14.)))
                            }),
                    )
                    .child(div().opacity(0.9).child(item.description))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.write(id, (!checked).into(), cx)
                    }))
                    .into_any_element()
            }
            Kind::Enum(options) => self.render_select(item, &value, options, cx),
        };

        v_flex()
            .id(SharedString::from(format!("setting-{id}")))
            .test_support()
            .aria_label(id)
            .relative()
            .pt(px(12.))
            .px(px(14.))
            .pb(px(if is_bool { 26. } else { 18. }))
            .text_color(foreground)
            // VS Code outlines the focused row by 1px; a transparent line
            // elsewhere keeps rows from moving when focus does.
            .border_1()
            .border_color(transparent_black())
            .when(focused, |this| {
                this.bg(colors::settings_focused_row())
                    .border_color(colors::focus_border())
            })
            .when(hovered && !focused, |this| this.bg(colors::settings_row_hover()))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let now = hovered.then_some(id);
                if this.hovered != now && (*hovered || this.hovered == Some(id)) {
                    this.hovered = now;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.focused_row = Some(id);
                    cx.notify();
                }),
            )
            .when(configured, |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("setting-{id}-modified")))
                        .test_support()
                        .absolute()
                        .left(px(5.))
                        .top(px(15.))
                        .bottom(px(if is_bool { 23. } else { 18. }))
                        .w(px(2.))
                        .bg(colors::settings_modified()),
                )
            })
            .child(self.render_gear(item, configured, hovered || focused, cx))
            .child(title)
            .when(!is_bool, |this| {
                this.child(
                    div()
                        .text_color(foreground)
                        .opacity(0.9)
                        .pb(px(8.))
                        .child(item.description),
                )
            })
            .child(control)
            .into_any_element()
    }

    /// VS Code's 「更多操作...」 gear left of a setting, shown on hover.
    fn render_gear(
        &self,
        item: &'static SettingItem,
        configured: bool,
        shown: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = cx.entity().downgrade();
        let id = item.id;
        div()
            .absolute()
            .left(px(-GEAR_SIZE))
            .top(px(8.))
            .size(px(GEAR_SIZE))
            .when(!shown, |this| this.opacity(0.))
            .child(
                Button::new(SharedString::from(format!("setting-{id}-more")))
                    .ghost()
                    .size(px(GEAR_SIZE))
                    .p_0()
                    .tooltip("更多操作...")
                    .child(Icon::new(Codicon::Gear).size(px(16.)))
                    .dropdown_menu(move |menu: PopupMenu, _, _| {
                        let reset = editor.clone();
                        let menu = if configured {
                            menu.item(PopupMenuItem::new("重置此设置").on_click(
                                move |_, _, cx| {
                                    if let Some(editor) = reset.upgrade() {
                                        editor.update(cx, |editor, cx| {
                                            editor.write(id, item.default_value(), cx)
                                        });
                                    }
                                },
                            ))
                        } else {
                            menu
                        };
                        menu.item(PopupMenuItem::new("复制设置 id").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(id.to_owned()))
                        }))
                        .item(
                            PopupMenuItem::new("将设置复制为 JSON 文本").on_click(
                                move |_, _, cx| {
                                    let values = *SettingsStore::global(cx).values();
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        item.json_text(&values),
                                    ))
                                },
                            ),
                        )
                    }),
            )
            .into_any_element()
    }

    /// An enum setting's drop-down: the value now, and the values with what
    /// each does when open.
    fn render_select(
        &self,
        item: &'static SettingItem,
        value: &Value,
        options: &'static [(&'static str, &'static str)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = value.as_str().unwrap_or_default().to_owned();
        let editor = cx.entity().downgrade();
        let id = item.id;
        Button::new(SharedString::from(format!("setting-{id}-control")))
            .w(px(SELECT_WIDTH))
            .h(px(SELECT_HEIGHT))
            .px(px(6.))
            .rounded(px(2.))
            .bg(colors::settings_control())
            .border_1()
            .border_color(colors::settings_control())
            .text_color(colors::settings_control_foreground())
            .child(
                h_flex()
                    .w_full()
                    .child(div().flex_1().child(current.clone()))
                    .child(Icon::new(Codicon::ChevronDown).size(px(16.))),
            )
            .dropdown_menu(move |menu: PopupMenu, _, cx| {
                let muted = cx.theme().muted_foreground;
                options.iter().fold(menu.min_w(px(SELECT_WIDTH)), |menu, (option, description)| {
                    let editor = editor.clone();
                    menu.item(
                        PopupMenuItem::element(move |_, _| {
                            v_flex()
                                .id(SharedString::from(format!("setting-{id}-option-{option}")))
                                .test_support()
                                .aria_label(*option)
                                .child(*option)
                                .child(div().text_size(px(12.)).text_color(muted).child(*description))
                        })
                        .checked(*option == current)
                        .on_click(move |_, _, cx| {
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.write(id, (*option).into(), cx)
                                });
                            }
                        }),
                    )
                })
            })
            .into_any_element()
    }
}

/// A group heading of the list, VS Code's `.settings-group-title-label`.
fn heading(label: &'static str, size: f32) -> AnyElement {
    div()
        .id(SharedString::from(format!("settings-heading-{label}")))
        .test_support()
        .aria_label(label)
        .pt(px(10.))
        .pr(px(10.))
        .pb(px(10.))
        .pl(px(15.))
        .text_size(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::settings_header_foreground())
        .child(label)
        .into_any_element()
}

impl Render for SettingsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let visible = self.visible(cx);
        let searching = !self.query.trim().is_empty();
        let count = searching.then_some(visible.len());
        let (children, headings) = self.render_list(&visible, cx);
        self.headings = headings;
        let toc = (!visible.is_empty()).then(|| self.render_toc(self.current_category(), &visible, cx));
        let list = (!visible.is_empty()).then(|| {
            div()
                .id("settings-list")
                .test_support()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .track_scroll(&self.list_scroll)
                .on_scroll(cx.listener(|this, _, _, cx| {
                    this.picked = None;
                    cx.notify();
                }))
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(MAX_WIDTH))
                        .mx_auto()
                        .px(px(CONTENT_INSET))
                        .children(children),
                )
                .into_any_element()
        });

        v_flex()
            .id("settings-editor")
            .key_context(SETTINGS_EDITOR_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .on_action(cx.listener(|this, _: &FocusSettingsSearch, window, cx| {
                this.focus_search(window, cx)
            }))
            .child(self.render_header(count, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mt(px(14.))
                    .child(container_query(move |size, _, _| {
                        let narrow = size.width < px(NARROW_WIDTH);
                        match (list, toc) {
                            (Some(list), Some(toc)) if !narrow => {
                                h_flex().size_full().child(toc).child(list).into_any_element()
                            }
                            (Some(list), _) => div().size_full().flex().child(list).into_any_element(),
                            (None, _) => div().into_any_element(),
                        }
                    })),
            )
    }
}
