//! The Source Control side bar: title actions, the commit message and button,
//! then the Merge Changes, Staged Changes and Changes groups.
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Textarea,
    menu::{DropdownMenu as _, PopupMenu},
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{TestSupportExt as _, prelude::FluentBuilder as _, *};

use super::{Repository, Workbench, editor_group::TabKey, paths_of};
use crate::{
    AbortMerge, AbortRebase, Checkout, Commit, CommitAmend, CommitAndPush, CommitAndSync,
    CreateBranch, CreateBranchFrom, DeleteBranch, DeleteRemoteBranch, DiscardAll, Fetch,
    MergeBranch, PublishBranch, Pull, Push, RebaseBranch, RenameBranch, StageAll, Sync, UnstageAll,
    codicons::Codicon,
    file_icons,
    git::{ChangeGroup, ChangedFile, Head, InProgress, StatusLetter},
    settings::{self, SettingsStore},
    theme::colors,
};

const ROW_HEIGHT: f32 = 22.;
/// Left inset of the message field and the button under it, as VS Code
/// draws them.
const INSET_LEFT: f32 = 19.;
/// Right inset of the title actions, the message field and the button: 12px
/// from the side bar's edge in VS Code, which has no border there; ours sits
/// inside a 1px border.
const INSET_RIGHT: f32 = 11.;
/// VS Code's codicons are 16px in every action bar and button.
const ICON_SIZE: f32 = 16.;
/// A title action: the codicon with `.action-label`'s 3px padding.
pub(super) const TITLE_ACTION_SIZE: f32 = 22.;
/// A row action: the codicon with the source control view's 2px padding.
const ROW_ACTION_SIZE: f32 = 20.;
/// The button under the message field as the user's VS Code draws it.
const TEXT_BUTTON_HEIGHT: f32 = 24.;

impl ChangeGroup {
    fn title(self) -> &'static str {
        match self {
            ChangeGroup::Merge => "合并更改",
            ChangeGroup::Staged => "暂存的更改",
            ChangeGroup::Changes => "更改",
        }
    }

    /// A stable name for element ids and tests.
    pub(super) fn key(self) -> &'static str {
        match self {
            ChangeGroup::Merge => "merge",
            ChangeGroup::Staged => "staged",
            ChangeGroup::Changes => "changes",
        }
    }

    /// VS Code's `hideWhenEmpty`: the git extension sets it on the merge and
    /// staged groups, never on the working tree's, which stays as 「更改 0」
    /// so that staging everything visibly empties it.
    fn hides_when_empty(self) -> bool {
        match self {
            ChangeGroup::Merge | ChangeGroup::Staged => true,
            ChangeGroup::Changes => false,
        }
    }
}

impl StatusLetter {
    /// VS Code's `gitDecoration.*` color for the letter.
    pub(super) fn color(self) -> Hsla {
        match self {
            StatusLetter::Modified => colors::modified(),
            StatusLetter::Added => colors::added(),
            StatusLetter::Deleted => colors::deleted(),
            StatusLetter::Renamed | StatusLetter::Copied | StatusLetter::Untracked => {
                colors::untracked()
            }
            StatusLetter::Conflicted => colors::conflicting(),
        }
    }
}

/// The button under the commit message, chosen as VS Code's source control
/// action button is: the next thing the repository needs. A merge or rebase
/// that stopped comes first, then changes to commit — the staged ones, and
/// the unstaged ones unless the user chose never to commit them all at once
/// (「从不」 turns `git.suggestSmartCommit` off); with none, a branch
/// without an upstream offers to publish, and one that differs from its
/// upstream offers to sync. With nothing left to do, the commit button
/// stays, disabled, as VS Code's does.
enum ActionButton {
    Continue { unresolved: bool },
    Commit { can_commit: bool },
    Publish,
    Sync { behind: u32, ahead: u32 },
}

impl ActionButton {
    fn of(repository: &Repository, settings: &settings::Values) -> Self {
        if repository
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.in_progress.is_some())
        {
            return ActionButton::Continue {
                unresolved: repository.group_files(ChangeGroup::Merge).next().is_some(),
            };
        }
        let has_staged = repository.group_files(ChangeGroup::Staged).next().is_some();
        let commits_unstaged = settings.enable_smart_commit || settings.suggest_smart_commit;
        let has_unstaged = repository
            .group_files(ChangeGroup::Changes)
            .next()
            .is_some();
        if has_staged || (commits_unstaged && has_unstaged) {
            return ActionButton::Commit { can_commit: true };
        }
        let nothing_to_commit = ActionButton::Commit { can_commit: false };
        let Some(branch) = repository.branch() else {
            return nothing_to_commit;
        };
        match (&branch.head, &branch.upstream) {
            (Head::Branch(_), None) => ActionButton::Publish,
            (_, Some(upstream)) if upstream.behind > 0 || upstream.ahead > 0 => {
                ActionButton::Sync {
                    behind: upstream.behind,
                    ahead: upstream.ahead,
                }
            }
            _ => nothing_to_commit,
        }
    }
}

/// Such as `同步更改 8↓ 2↑`, leaving out a direction with nothing to move.
fn sync_label(behind: u32, ahead: u32) -> String {
    let mut label = String::from("同步更改");
    if behind > 0 {
        label.push_str(&format!(" {behind}↓"));
    }
    if ahead > 0 {
        label.push_str(&format!(" {ahead}↑"));
    }
    label
}

/// What the 「…」 menu needs to know to enable its items.
#[derive(Clone, Copy)]
struct MoreMenuState {
    busy: bool,
    has_commits: bool,
    on_branch: bool,
    has_upstream: bool,
    in_progress: Option<InProgress>,
    has_changes: bool,
    has_staged: bool,
}

impl MoreMenuState {
    fn of(repository: &Repository) -> Self {
        let branch = repository.branch();
        Self {
            busy: repository.is_busy(),
            has_commits: branch.is_some_and(|branch| branch.has_commits),
            on_branch: branch.is_some_and(|branch| matches!(branch.head, Head::Branch(_))),
            has_upstream: branch.is_some_and(|branch| branch.upstream.is_some()),
            in_progress: repository
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.in_progress),
            has_changes: repository
                .group_files(ChangeGroup::Changes)
                .next()
                .is_some(),
            has_staged: repository.group_files(ChangeGroup::Staged).next().is_some(),
        }
    }
}

/// VS Code's 「…」 menu of the Source Control title, with the items this app
/// implements: the most used actions, then the Commit, Changes, Pull/Push and
/// Branch submenus.
fn more_menu(
    menu: PopupMenu,
    state: MoreMenuState,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let busy = state.busy;
    let submenu = |build: fn(PopupMenu, MoreMenuState) -> PopupMenu| {
        let focus = focus.clone();
        move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            build(menu.action_context(focus.clone()), state)
        }
    };
    menu.action_context(focus.clone())
        .menu_with_disabled("拉取", Box::new(Pull), busy)
        .menu_with_disabled(
            if state.on_branch && !state.has_upstream {
                "发布分支"
            } else {
                "推送"
            },
            Box::new(Push),
            busy,
        )
        .menu_with_disabled("签出到...", Box::new(Checkout), busy)
        .menu_with_disabled("抓取", Box::new(Fetch), busy)
        .separator()
        .submenu("提交", window, cx, submenu(commit_submenu))
        .submenu("更改", window, cx, submenu(changes_submenu))
        .submenu("拉取，推送", window, cx, submenu(pull_push_submenu))
        .submenu("分支", window, cx, submenu(branch_submenu))
}

fn commit_submenu(menu: PopupMenu, state: MoreMenuState) -> PopupMenu {
    menu.menu_with_disabled("提交", Box::new(Commit), state.busy)
        .menu_with_disabled(
            "提交(修改)",
            Box::new(CommitAmend),
            state.busy || !state.has_commits,
        )
        .when(state.in_progress == Some(InProgress::Rebase), |menu| {
            menu.separator()
                .menu_with_disabled("中止变基", Box::new(AbortRebase), state.busy)
        })
}

fn changes_submenu(menu: PopupMenu, state: MoreMenuState) -> PopupMenu {
    menu.menu_with_disabled(
        "暂存所有更改",
        Box::new(StageAll),
        state.busy || !state.has_changes,
    )
    .menu_with_disabled(
        "取消暂存所有更改",
        Box::new(UnstageAll),
        state.busy || !state.has_staged,
    )
    .menu_with_disabled(
        "放弃所有更改",
        Box::new(DiscardAll),
        state.busy || !state.has_changes,
    )
}

fn pull_push_submenu(menu: PopupMenu, state: MoreMenuState) -> PopupMenu {
    menu.menu_with_disabled("同步", Box::new(Sync), state.busy)
        .separator()
        .menu_with_disabled("拉取", Box::new(Pull), state.busy)
        .separator()
        .menu_with_disabled("推送", Box::new(Push), state.busy)
        .separator()
        .menu_with_disabled("抓取", Box::new(Fetch), state.busy)
}

fn branch_submenu(menu: PopupMenu, state: MoreMenuState) -> PopupMenu {
    let busy = state.busy;
    menu.menu_with_disabled("合并...", Box::new(MergeBranch), busy)
        .menu_with_disabled("变基分支…", Box::new(RebaseBranch), busy)
        .when(state.in_progress == Some(InProgress::Merge), |menu| {
            menu.menu_with_disabled("中止合并", Box::new(AbortMerge), busy)
        })
        .separator()
        .menu_with_disabled("创建分支...", Box::new(CreateBranch), busy)
        .menu_with_disabled(
            "从现有来源创建新的分支...",
            Box::new(CreateBranchFrom),
            busy,
        )
        .separator()
        .menu_with_disabled(
            "重命名分支...",
            Box::new(RenameBranch),
            busy || !state.on_branch,
        )
        .menu_with_disabled("删除分支...", Box::new(DeleteBranch), busy)
        .menu_with_disabled("删除远程分支...", Box::new(DeleteRemoteBranch), busy)
        .separator()
        .menu_with_disabled(
            "发布分支...",
            Box::new(PublishBranch),
            busy || !state.on_branch || state.has_upstream,
        )
}

/// The element id of a changed file's row in the side bar.
pub(super) fn file_row_id(group: ChangeGroup, path: &str) -> SharedString {
    format!("scm-file-{}-{path}", group.key()).into()
}

impl Workbench {
    pub(super) fn render_scm_panel(
        &self,
        repository: &Repository,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (sidebar, sidebar_border) = {
            let theme = cx.theme();
            (theme.sidebar, theme.sidebar_border)
        };
        let busy = repository.is_busy();
        let focus = self.focus_handle.clone();

        let actions = h_flex()
                    .flex_none()
                    .pl(px(4.))
                    .pr(px(INSET_RIGHT))
                    // `.action-item`'s 4px margin.
                    .gap(px(4.))
                    .child(
                        icon_button("scm-commit", Codicon::Check, TITLE_ACTION_SIZE, busy)
                            .tooltip("提交 (⌘Enter)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.commit(false, super::AfterCommit::Nothing, window, cx)
                            })),
                    )
                    .child(
                        icon_button("scm-refresh", Codicon::Refresh, TITLE_ACTION_SIZE, false)
                            .tooltip("刷新")
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    )
                    .child(
                        icon_button("scm-more", Codicon::Ellipsis, TITLE_ACTION_SIZE, false)
                            .tooltip("更多操作…")
                            .dropdown_menu({
                                let focus = focus.clone();
                                let state = MoreMenuState::of(repository);
                                move |menu: PopupMenu, window, cx| {
                                    more_menu(menu, state, &focus, window, cx)
                                }
                            }),
                    );
        let title = self.side_bar_title(
            repository,
            "SOURCE CONTROL",
            Some(actions.into_any_element()),
            cx,
        );

        let message = div()
            .key_context("CommitMessage")
            .pt(px(4.))
            .pl(px(INSET_LEFT))
            .pr(px(INSET_RIGHT))
            .child(
                // VS Code's input: the 13px workbench font on 20px lines,
                // 26px tall with one line.
                Textarea::new(&repository.message)
                    .small()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .bg(colors::input_background())
                    .aria_label("提交信息"),
            );

        let action_button = div()
            .pl(px(INSET_LEFT))
            .pr(px(INSET_RIGHT))
            .pt(px(10.))
            .child(self.render_action_button(repository, cx));

        let list_focused = self.scm_list_focus.is_focused(window);
        let mut groups = Vec::new();
        // The groups count what git reported; before its first answer there
        // is nothing to count, and 「更改 0」 would claim a clean tree.
        if repository.snapshot.is_some() {
            for group in [
                ChangeGroup::Merge,
                ChangeGroup::Staged,
                ChangeGroup::Changes,
            ] {
                groups.extend(self.render_group(repository, group, list_focused, cx));
            }
        }

        v_flex()
            .size_full()
            .bg(sidebar)
            .border_r_1()
            .border_color(sidebar_border)
            .child(title)
            .child(message)
            .child(action_button)
            .child(
                div()
                    .id("scm-groups")
                    .track_focus(&self.scm_list_focus)
                    .flex_1()
                    .min_h_0()
                    .pt(px(6.))
                    .overflow_y_scroll()
                    .children(groups),
            )
    }

    /// The button under the commit message, as [`ActionButton`] picks it.
    fn render_action_button(&self, repository: &Repository, cx: &mut Context<Self>) -> AnyElement {
        let busy = repository.is_busy();
        let has_commits = repository.branch().is_some_and(|branch| branch.has_commits);
        let focus = self.focus_handle.clone();
        match ActionButton::of(repository, SettingsStore::global(cx).values()) {
            ActionButton::Commit { can_commit } => {
                let disabled = busy || !can_commit;
                h_flex()
                    .w_full()
                    .child(
                        text_button("commit", button_icon(Codicon::Check), "提交")
                            .primary()
                            .flex_1()
                            .rounded_r_none()
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.commit(false, super::AfterCommit::Nothing, window, cx)
                            })),
                    )
                    .child(
                        menu_half("commit-menu", disabled, cx)
                            .accessibility_label("提交操作")
                            .dropdown_menu(move |menu, _, _| {
                                menu.action_context(focus.clone())
                                    .menu("提交", Box::new(Commit))
                                    .menu_with_disabled(
                                        "提交(修改)",
                                        Box::new(CommitAmend),
                                        !has_commits,
                                    )
                                    .separator()
                                    .menu("提交并推送", Box::new(CommitAndPush))
                                    .menu("提交并同步", Box::new(CommitAndSync))
                            }),
                    )
                    .into_any_element()
            }
            ActionButton::Continue { unresolved } => {
                text_button("continue", button_icon(Codicon::Check), "继续")
                    .primary()
                    .w_full()
                    .disabled(busy || unresolved)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.continue_in_progress(window, cx)),
                    )
                    .into_any_element()
            }
            ActionButton::Publish => text_button(
                "publish-branch",
                syncing_icon(Codicon::CloudUpload, repository.is_syncing()),
                "发布分支",
            )
            .secondary()
            .w_full()
            .loading(repository.is_syncing())
            .disabled(busy)
            .on_click(cx.listener(|this, _, window, cx| this.publish(window, cx)))
            .into_any_element(),
            ActionButton::Sync { behind, ahead } => text_button(
                "sync-changes",
                syncing_icon(Codicon::Sync, repository.is_syncing()),
                sync_label(behind, ahead),
            )
            .secondary()
            .w_full()
            .loading(repository.is_syncing())
            .disabled(busy)
            .on_click(cx.listener(|this, _, window, cx| this.sync(window, cx)))
            .into_any_element(),
        }
    }

    fn render_group(
        &self,
        repository: &Repository,
        group: ChangeGroup,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let files: Vec<ChangedFile> = repository.group_files(group).cloned().collect();
        if files.is_empty() && group.hides_when_empty() {
            return None;
        }
        let list_hover = cx.theme().list_hover;
        let collapsed = repository.collapsed_groups.contains(&group);
        let busy = repository.is_busy();
        let header_id: SharedString = format!("scm-group-{}", group.key()).into();
        let actions = self
            .is_scm_row_hovered(&header_id)
            .then(|| self.group_actions(group, &files, busy, cx));

        let header = h_flex()
            .id(header_id.clone())
            .test_support()
            .aria_label(group.title())
            .on_hover(self.scm_row_hover_listener(header_id, cx))
            .h(px(ROW_HEIGHT))
            .pl(px(10.))
            .pr(px(8.))
            .gap(px(4.))
            .cursor_pointer()
            .hover(move |style| style.bg(list_hover))
            .child(
                Icon::new(if collapsed {
                    Codicon::ChevronRight
                } else {
                    Codicon::ChevronDown
                })
                .size(px(16.)),
            )
            .child(div().flex_1().min_w_0().truncate().child(group.title()))
            .children(actions.map(row_actions))
            .child(
                div()
                    .ml(px(4.))
                    .px(px(6.))
                    .min_w(px(18.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(colors::badge_background())
                    .text_color(colors::badge_foreground())
                    .text_size(px(11.))
                    .child(files.len().to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(group, cx)));

        let rows = (!collapsed).then(|| {
            files
                .iter()
                .map(|changed| self.render_file_row(changed, busy, list_focused, cx))
                .collect::<Vec<_>>()
        });
        Some(
            v_flex()
                .child(header)
                .children(rows.into_iter().flatten())
                .into_any_element(),
        )
    }

    fn is_scm_row_hovered(&self, row: &SharedString) -> bool {
        self.scm_hovered_row.as_ref() == Some(row)
    }

    /// Keeps the row under the pointer, whose actions show as VS Code's
    /// `:hover` shows them.
    fn scm_row_hover_listener(
        &self,
        row: SharedString,
        cx: &mut Context<Self>,
    ) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered {
                this.scm_hovered_row = Some(row.clone());
            } else if this.is_scm_row_hovered(&row) {
                // The next row may report entering before this one reports
                // leaving; only the row still recorded clears it.
                this.scm_hovered_row = None;
            } else {
                return;
            }
            cx.notify();
        })
    }

    /// The buttons a group header shows on hover.
    fn group_actions(
        &self,
        group: ChangeGroup,
        files: &[ChangedFile],
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let id = |name: &str| SharedString::from(format!("{name}-{}", group.key()));
        let all_paths: Vec<Vec<String>> = files.iter().map(paths_of).collect();
        let mut actions = Vec::new();
        match group {
            ChangeGroup::Staged => actions.push(
                row_button(id("unstage-all"), Codicon::Remove, "取消暂存所有更改", busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.unstage(all_paths.clone(), window, cx)
                    }))
                    .into_any_element(),
            ),
            ChangeGroup::Changes => {
                let files = files.to_vec();
                actions.push(
                    row_button(
                        id("discard-all"),
                        Codicon::Discard,
                        "放弃所有更改",
                        busy,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.discard(files.clone(), window, cx)
                    }))
                    .into_any_element(),
                );
                actions.push(
                    row_button(id("stage-all"), Codicon::Add, "暂存所有更改", busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.stage(all_paths.clone(), window, cx)
                        }))
                        .into_any_element(),
                );
            }
            ChangeGroup::Merge => actions.push(
                row_button(id("stage-all"), Codicon::Add, "暂存所有更改", busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.stage(all_paths.clone(), window, cx)
                    }))
                    .into_any_element(),
            ),
        }
        actions
    }

    fn render_file_row(
        &self,
        changed: &ChangedFile,
        busy: bool,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (list_active, list_hover, muted) = {
            let theme = cx.theme();
            let list_active = if list_focused {
                colors::list_focused_selection()
            } else {
                theme.list_active
            };
            (list_active, theme.list_hover, theme.muted_foreground)
        };
        let group = changed.group;
        let path = changed.file.path().clone();
        let selected = self
            .active_diff()
            .is_some_and(|active| active.group == group && active.file.path() == &path);
        let (directory, name) = match path.rsplit_once('/') {
            Some((directory, name)) => (directory.to_owned(), name.to_owned()),
            None => (String::new(), path.to_string()),
        };
        let row_id = file_row_id(group, &path);
        let rename = changed
            .file
            .original_path()
            .filter(|original| original.as_str() != path.as_str())
            .map(|original| SharedString::from(format!("{original} → {path}")));
        // VS Code shows a row's actions while the pointer is over it or it
        // is the focused row, and leaves them out of the layout otherwise,
        // so a narrow side bar gives their room to the label.
        let actions = (selected || self.is_scm_row_hovered(&row_id))
            .then(|| self.file_actions(changed, busy, cx));
        let open_path = path.clone();

        h_flex()
            .id(row_id.clone())
            .test_support()
            .aria_label(path.clone())
            .on_hover(self.scm_row_hover_listener(row_id.clone(), cx))
            .relative()
            .h(px(ROW_HEIGHT))
            .pl(px(16.))
            .pr(px(12.))
            .gap(px(6.))
            .cursor_pointer()
            .when(selected, |this| this.bg(list_active))
            // `list.focusOutline`, drawn inside the row as VS Code's
            // `outline-offset: -1px` does, so it shifts nothing.
            .when(selected && list_focused, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .border_1()
                        .border_color(colors::list_focus_outline()),
                )
            })
            .when(!selected, |this| {
                this.hover(move |style| style.bg(list_hover))
            })
            .child(img(file_icons::icon_for(&path)).size(px(16.)).flex_none())
            // As VS Code's label reads on one line cut at its end, the
            // folder gives way first, then the name.
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .gap(px(6.))
                    .child(div().flex_none().max_w_full().truncate().child(name))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(directory),
                    ),
            )
            .children(actions.map(row_actions))
            .child(
                div()
                    .w(px(12.))
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(changed.letter.color())
                    .child(changed.letter.as_str()),
            )
            .when_some(rename, |this, rename| {
                this.tooltip(move |window, cx| Tooltip::new(rename.clone()).build(window, cx))
            })
            // A click opens a preview tab, a double click one for good.
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                window.focus(&this.scm_list_focus, cx);
                let key = TabKey::Diff {
                    group,
                    path: open_path.clone(),
                };
                this.open_tab(key, event.click_count() < 2, window, cx)
            }))
            .into_any_element()
    }

    /// The buttons a file row shows on hover, by group as in VS Code.
    fn file_actions(
        &self,
        changed: &ChangedFile,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let key = format!("{}-{}", changed.group.key(), changed.file.path());
        let id = |name: &str| SharedString::from(format!("{name}-{key}"));
        let paths = paths_of(changed);
        let mut actions = Vec::new();
        if changed.letter != StatusLetter::Deleted {
            let path = changed.file.path().clone();
            actions.push(
                row_button(
                    id("open"),
                    Codicon::GoToFile,
                    "打开文件",
                    false,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_changed_file(&path, window, cx)
                }))
                .into_any_element(),
            );
        }
        match changed.group {
            ChangeGroup::Staged => actions.push(
                row_button(id("unstage"), Codicon::Remove, "取消暂存更改", busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.unstage(vec![paths.clone()], window, cx)
                    }))
                    .into_any_element(),
            ),
            ChangeGroup::Changes => {
                let file = changed.clone();
                actions.push(
                    row_button(id("discard"), Codicon::Discard, "放弃更改", busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.discard(vec![file.clone()], window, cx)
                        }))
                        .into_any_element(),
                );
                actions.push(
                    row_button(id("stage"), Codicon::Add, "暂存更改", busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.stage(vec![paths.clone()], window, cx)
                        }))
                        .into_any_element(),
                );
            }
            ChangeGroup::Merge => actions.push(
                row_button(id("stage"), Codicon::Add, "暂存更改", busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.stage(vec![paths.clone()], window, cx)
                    }))
                    .into_any_element(),
            ),
        }
        actions
    }
}

fn row_button(
    id: SharedString,
    icon: impl Into<Icon>,
    tooltip: &'static str,
    disabled: bool,
) -> Button {
    icon_button(id, icon, ROW_ACTION_SIZE, disabled).tooltip(tooltip)
}

fn row_actions(actions: Vec<AnyElement>) -> impl IntoElement {
    h_flex().flex_none().children(actions)
}

/// A VS Code action bar item: the 16px icon centered in a square, in
/// `icon.foreground`, at 60% opacity when disabled (`.action-item.disabled
/// .action-label.icon`). The icon goes in as a child, since `Button::icon`
/// sizes it from the button's size; its own color also keeps the button's
/// disabled color off it, hence the opacity.
pub(super) fn icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    size: f32,
    disabled: bool,
) -> Button {
    Button::new(id)
        .ghost()
        .size(px(size))
        .p_0()
        .disabled(disabled)
        .when(disabled, |this| this.opacity(0.6))
        .child(
            icon.into()
                .size(px(ICON_SIZE))
                .text_color(colors::icon_foreground()),
        )
}

fn button_icon(icon: impl Into<Icon>) -> AnyElement {
    icon.into().size(px(ICON_SIZE)).into_any_element()
}

/// `icon` until a sync runs, then the turning sync icon.
fn syncing_icon(icon: impl Into<Icon>, syncing: bool) -> AnyElement {
    if syncing {
        Spinner::new()
            .icon(Icon::new(Codicon::Sync))
            .with_size(Size::Size(px(ICON_SIZE)))
            .into_any_element()
    } else {
        button_icon(icon)
    }
}

/// The menu half of VS Code's `.monaco-button-dropdown`: its 1px separator,
/// inset 4px top and bottom in `button.separator` (the button foreground at
/// 40%, dimmed like the rest when disabled), then the 16px chevron with 4px
/// either side.
fn menu_half(id: impl Into<ElementId>, disabled: bool, cx: &App) -> Button {
    let separator = cx
        .theme()
        .primary_foreground
        .opacity(if disabled { 0.4 * 0.4 } else { 0.4 });
    Button::new(id)
        .primary()
        .rounded_l_none()
        .h(px(TEXT_BUTTON_HEIGHT))
        .w(px(1. + 4. + ICON_SIZE + 4.))
        .pl_0()
        .pr(px(4.))
        .disabled(disabled)
        // One child, as `Button` spaces its children wider than 4px.
        .child(
            h_flex()
                .gap(px(4.))
                .child(div().w(px(1.)).h(px(TEXT_BUTTON_HEIGHT - 8.)).bg(separator))
                .child(button_icon(Codicon::ChevronDown)),
        )
}

/// VS Code's `.monaco-text-button`: 4px 8px padding, the 16px codicon with
/// its 0.2em margin, then 12px text cut short when the side bar is narrow.
/// Icon and label go in as one child, since `Button` sizes both from its own
/// size, smaller than these, and spaces its children wider.
fn text_button(
    id: impl Into<ElementId>,
    icon: AnyElement,
    label: impl Into<SharedString>,
) -> Button {
    let label = label.into();
    Button::new(id)
        .h(px(TEXT_BUTTON_HEIGHT))
        .px(px(8.))
        .accessibility_label(label.clone())
        .child(
            h_flex()
                .min_w_0()
                .gap(px(3.2))
                .child(icon)
                .child(div().min_w_0().truncate().text_size(px(12.)).child(label)),
        )
}
