//! The window: the welcome page until a repository is open, then VS Code's
//! Source Control layout — the side bar, the diff editor and the status bar.
//! Each area renders from its own file; they share this one state.
mod activity_bar;
mod branches;
mod editor;
mod editor_group;
mod explorer;
mod explorer_panel;
mod file_buffer;
mod outside_files;
mod quick_input;
mod relative_time;
mod scm_panel;
mod settings_editor;
mod status_bar;
mod user_settings;
mod welcome;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_channel::oneshot;
use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    button::Button,
    diff::{DiffMode, DiffState},
    input::TextareaState,
    notification::Notification,
    h_flex,
    resizable::{h_resizable, resizable_panel},
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::{
    APP_NAME, AbortMerge, AbortRebase, About, Checkout, CloseActiveTab, CloseFolder, Commit,
    Quit,
    CommitAmend,
    CommitAndPush, CommitAndSync, CreateBranch, CreateBranchFrom, DeleteBranch, DeleteRemoteBranch,
    DiscardAll, Fetch, MergeBranch, OpenFolder, OpenTerminal, PublishBranch, Pull, Push,
    RebaseBranch, Refresh, RenameBranch, Save, StageAll, Sync, UnstageAll,
    git::{self, BranchStatus, ChangeGroup, ChangedFile, GitError, Head, Snapshot},
    workbench::{
        editor_group::{EditorGroup, Opened, TabKey},
        explorer::{Decorations, Explorer},
        file_buffer::{FileBuffer, FileBufferEvent, SaveError},
    },
    recent::RecentFolders,
    settings::SettingsStore,
    terminal,
    watcher::{self, Batch},
};

pub use settings_editor::SETTINGS_EDITOR_CONTEXT;

/// The key context of the commit message field, for its cmd-enter binding.
pub const COMMIT_MESSAGE_CONTEXT: &str = "CommitMessage > Input";

pub struct Workbench {
    focus_handle: FocusHandle,
    /// The changes list in the sidebar. Its selected row shows VS Code's
    /// focused selection while the list has focus.
    scm_list_focus: FocusHandle,
    /// The side bar row or group header under the pointer, which shows its
    /// actions.
    scm_hovered_row: Option<SharedString>,
    /// The explorer's tree, for its keys and its focused selection.
    explorer_focus: FocusHandle,
    /// Whether the pointer is over the explorer's tree, which then shows its
    /// indent guides.
    explorer_hovered: bool,
    recent: RecentFolders,
    /// Whether the notice about a settings file that does not parse is up.
    settings_error_shown: bool,
    /// The editor area. It belongs to the window, not to the repository:
    /// it is there with no folder open, and a tab that shows no file of the
    /// repository stays when another folder opens.
    editors: Editors,
    /// Watches the folders of files open from outside the repository.
    outside_watcher: Option<outside_files::OutsideWatcher>,
    /// Whether the welcome page lists every recent folder or only the first ten.
    show_all_recent: bool,
    repository: Option<Repository>,
    /// Why the last folder could not be opened, shown on the welcome page.
    open_error: Option<SharedString>,
    opening: Task<()>,
    /// The picker or input box dropped from the top, with its dismissal
    /// subscription.
    quick_input: Option<(quick_input::QuickInput, Subscription)>,
    _activation: Subscription,
    _settings_observation: Subscription,
}

/// The view the side bar shows, picked in the activity bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideView {
    Explorer,
    SourceControl,
}

struct Repository {
    root: PathBuf,
    side_view: SideView,
    explorer: Explorer,
    explorer_scroll: UniformListScrollHandle,
    /// Whether the folder's section in the explorer is folded away.
    explorer_collapsed: bool,
    /// The git colors of the explorer, from the latest snapshot.
    decorations: Decorations,
    /// Holds the file of one diff tab at a time, the active one's or the
    /// last one shown; every diff tab shares it.
    diff: Entity<DiffState>,
    /// The patch in `diff` now. Putting the same patch in again would reset
    /// its scroll position, expanded folds and selection.
    shown_patch: Option<Arc<str>>,
    message: Entity<TextareaState>,
    load_state: LoadState,
    snapshot: Option<Snapshot>,
    collapsed_groups: Vec<ChangeGroup>,
    /// The repository change in progress; only one runs at a time.
    operation: Option<RunningOperation>,
    /// Replacing this task cancels a refresh still in flight, so only the
    /// latest refresh can ever update the view.
    refresh_task: Task<()>,
    /// Owns the file watcher and takes in its batches; dropped with the
    /// repository, which stops the watching.
    _watching: Task<()>,
    _diff_observation: Subscription,
}

/// The tabs of the editor area and what its file tabs hold.
#[derive(Default)]
struct Editors {
    group: EditorGroup,
    /// The buffer of every file tab, by its absolute path.
    buffers: HashMap<Arc<Path>, OpenBuffer>,
    /// The tab under the pointer, which shows its close button.
    hovered_tab: Option<TabKey>,
    /// The tab bar's horizontal scrolling, which keeps the active tab in view.
    tab_scroll: ScrollHandle,
    /// The settings page, while its tab is open.
    settings: Option<OpenSettingsEditor>,
}

struct OpenSettingsEditor {
    editor: Entity<settings_editor::SettingsEditor>,
    _subscription: Subscription,
}

struct OpenBuffer {
    buffer: Entity<FileBuffer>,
    _subscriptions: [Subscription; 2],
}

impl Editors {
    /// The buffer of the active tab, when that is a file tab.
    fn active_buffer(&self) -> Option<&Entity<FileBuffer>> {
        match self.group.active()? {
            TabKey::File(path) => self.buffers.get(path).map(|open| &open.buffer),
            TabKey::Diff { .. } | TabKey::Settings => None,
        }
    }

    /// Closes `key` and drops what it held.
    fn close(&mut self, key: &TabKey) {
        self.group.close(key);
        self.tab_gone(key);
    }

    /// Drops what a closed or replaced tab held.
    fn tab_gone(&mut self, key: &TabKey) {
        match key {
            TabKey::File(path) => {
                self.buffers.remove(path);
            }
            TabKey::Settings => self.settings = None,
            TabKey::Diff { .. } => {}
        }
        if self.hovered_tab.as_ref() == Some(key) {
            self.hovered_tab = None;
        }
    }

    /// Scrolls the active tab into view.
    fn scroll_to_active(&self) {
        if let Some(ix) = self
            .group
            .active()
            .and_then(|active| self.group.tabs().iter().position(|tab| tab == active))
        {
            self.tab_scroll.scroll_to_item(ix);
        }
    }
}

enum LoadState {
    Loading,
    Loaded,
    Failed(SharedString),
}

struct RunningOperation {
    operation: Operation,
    _task: Task<()>,
}

/// Repository changes, named for their failure notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Stage,
    Unstage,
    Discard,
    Commit,
    Pull,
    Push,
    Publish,
    Sync,
    Fetch,
    Checkout,
    CreateBranch,
    RenameBranch,
    DeleteBranch,
    DeleteRemoteBranch,
    Merge,
    Rebase,
    ContinueRebase,
    AbortMerge,
    AbortRebase,
}

impl Operation {
    fn label(self) -> &'static str {
        match self {
            Operation::Stage => "暂存",
            Operation::Unstage => "取消暂存",
            Operation::Discard => "放弃更改",
            Operation::Commit => "提交",
            Operation::Pull => "拉取",
            Operation::Push => "推送",
            Operation::Publish => "发布分支",
            Operation::Sync => "同步",
            Operation::Fetch => "抓取",
            Operation::Checkout => "签出",
            Operation::CreateBranch => "创建分支",
            Operation::RenameBranch => "重命名分支",
            Operation::DeleteBranch => "删除分支",
            Operation::DeleteRemoteBranch => "删除远程分支",
            Operation::Merge => "合并",
            Operation::Rebase => "变基",
            Operation::ContinueRebase => "继续变基",
            Operation::AbortMerge => "中止合并",
            Operation::AbortRebase => "中止变基",
        }
    }

    fn is_remote(self) -> bool {
        matches!(
            self,
            Operation::Pull
                | Operation::Push
                | Operation::Publish
                | Operation::Sync
                | Operation::Fetch
                | Operation::DeleteRemoteBranch
        )
    }
}

/// What follows a successful commit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterCommit {
    Nothing,
    Push,
    Sync,
}

impl Repository {
    fn files(&self) -> &[ChangedFile] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.files.as_slice())
    }

    fn branch(&self) -> Option<&BranchStatus> {
        self.snapshot.as_ref().map(|snapshot| &snapshot.branch)
    }

    fn file(&self, group: ChangeGroup, path: &str) -> Option<&ChangedFile> {
        self.files()
            .iter()
            .find(|changed| changed.group == group && changed.file.path().as_ref() == path)
    }

    /// The changed file `active` shows, when that is a diff tab.
    fn diff_of(&self, active: Option<&TabKey>) -> Option<&ChangedFile> {
        match active? {
            TabKey::Diff { group, path } => self.file(*group, path),
            TabKey::File(_) | TabKey::Settings => None,
        }
    }

    /// `path` relative to the root, `/`-separated, when it is inside.
    fn relative(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        Some(
            relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    fn group_files(&self, group: ChangeGroup) -> impl Iterator<Item = &ChangedFile> {
        self.files()
            .iter()
            .filter(move |changed| changed.group == group)
    }

    fn is_busy(&self) -> bool {
        self.operation.is_some()
    }

    /// Whether the change in progress talks to a remote.
    fn is_syncing(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|running| running.operation.is_remote())
    }

    /// Puts the active diff tab's file in the diff unless its patch is there
    /// already. A file tab leaves the diff as it is, so switching back to the
    /// diff tab finds it where it was.
    fn show_active_diff(&mut self, active: Option<&TabKey>, cx: &mut App) {
        if matches!(active, Some(TabKey::File(_) | TabKey::Settings)) {
            return;
        }
        let active = self.diff_of(active);
        let patch = active.map(|changed| changed.patch.clone());
        if patch == self.shown_patch {
            return;
        }
        let files: Vec<_> = active
            .map(|changed| changed.file.clone())
            .into_iter()
            .collect();
        self.diff.update(cx, |diff, cx| diff.set_files(files, cx));
        self.shown_patch = patch;
    }
}

/// The paths an operation on `changed` must name: a rename has two.
fn paths_of(changed: &ChangedFile) -> Vec<String> {
    let path = changed.file.path().to_string();
    match changed.file.original_path() {
        Some(original) if original.as_str() != path => vec![path, original.to_string()],
        _ => vec![path],
    }
}

impl Workbench {
    pub fn new(
        initial_folder: Option<PathBuf>,
        recent: RecentFolders,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        // A picker closes when the window loses focus, as in VS Code. Changes
        // made elsewhere arrive through the file watcher, not on returning.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.dismiss_picker_on_blur(window, cx);
            }
        });
        // The window's close button asks about unsaved edits first.
        let view_handle = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(view) = view_handle.upgrade() else {
                return true;
            };
            view.update(cx, |view, cx| {
                if view.dirty_files(None, cx).is_empty() {
                    return true;
                }
                let confirmed = view.confirm_unsaved_files(window, cx);
                cx.spawn_in(window, async move |_, cx| {
                    if confirmed.await
                        && let Err(error) = cx.update(|window, _| window.remove_window())
                    {
                        log::warn!("window gone before it could close: {error:#}");
                    }
                })
                .detach();
                false
            })
        });
        let mut view = Self {
            focus_handle,
            scm_list_focus: cx.focus_handle(),
            scm_hovered_row: None,
            explorer_focus: cx.focus_handle(),
            explorer_hovered: false,
            recent,
            settings_error_shown: false,
            editors: Editors::default(),
            outside_watcher: None,
            show_all_recent: false,
            repository: None,
            open_error: None,
            opening: Task::ready(()),
            quick_input: None,
            _activation: activation,
            // Every setting is read from the store as the window renders.
            _settings_observation: cx.observe_global_in::<SettingsStore>(
                window,
                |this, window, cx| {
                    this.settings_error_changed(window, cx);
                    cx.notify();
                },
            ),
        };
        // A file broken before the start is told about once the window is up.
        cx.defer_in(window, |this, window, cx| this.settings_error_changed(window, cx));
        if let Some(folder) = initial_folder {
            view.open_folder(folder, window, cx);
        }
        view
    }

    fn prompt_open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开".into()),
        });
        self.opening = cx.spawn_in(window, async move |this, cx| {
            let folder = match selection.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => {
                    log::error!("folder prompt failed: {error:#}");
                    None
                }
            };
            let Some(folder) = folder else {
                return;
            };
            let opened =
                this.update_in(cx, |this, window, cx| this.open_folder(folder, window, cx));
            if let Err(error) = opened {
                log::warn!("view released before opening folder: {error:#}");
            }
        });
    }

    fn open_folder(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let resolve_root = cx.background_spawn(async move { git::repository_root(&folder) });
        self.opening = cx.spawn_in(window, async move |this, cx| {
            let root = resolve_root.await;
            let shown = this.update_in(cx, |this, window, cx| match root {
                Ok(root) => this.after_unsaved_files(
                    move |this, window, cx| this.show_repository(root, window, cx),
                    window,
                    cx,
                ),
                Err(error) => {
                    let message = SharedString::from(error.to_string());
                    this.after_unsaved_files(
                        move |this, window, cx| this.show_open_error(message, window, cx),
                        window,
                        cx,
                    )
                }
            });
            if let Err(error) = shown {
                log::warn!("view released before showing repository: {error:#}");
            }
        });
    }

    /// Opens a folder from the recent list, offering to forget it when it
    /// cannot be opened any more.
    fn open_recent(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let resolve_root = cx.background_spawn({
            let folder = folder.clone();
            async move {
                if folder.is_dir() {
                    git::repository_root(&folder).map_err(|error| error.to_string())
                } else {
                    Err("该文件夹已不存在".to_owned())
                }
            }
        });
        self.opening = cx.spawn_in(window, async move |this, cx| {
            let root = resolve_root.await;
            let shown = this.update_in(cx, |this, window, cx| match root {
                Ok(root) => this.after_unsaved_files(
                    move |this, window, cx| this.show_repository(root, window, cx),
                    window,
                    cx,
                ),
                Err(reason) => this.offer_to_forget(folder, reason, window, cx),
            });
            if let Err(error) = shown {
                log::warn!("view released before opening a recent folder: {error:#}");
            }
        });
    }

    fn offer_to_forget(
        &mut self,
        folder: PathBuf,
        reason: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("无法打开 {}", folder.display()),
            Some(&format!("{reason}。是否从最近打开的列表中移除？")),
            &["移除", "保留"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            if let Err(error) = this.update(cx, |this, cx| {
                this.recent.remove(&folder);
                cx.notify();
            }) {
                log::warn!("view released before forgetting a folder: {error:#}");
            }
        })
        .detach();
    }

    /// Watches the repository at `root` and takes in what changes.
    ///
    /// Not in unit tests: their scheduler is deterministic and refuses
    /// wake-ups from the watcher's thread, so they hand batches to
    /// `files_changed` themselves; the screenshot tests run the real one.
    #[cfg(test)]
    fn watch(_: PathBuf, _: &mut Window, _: &mut Context<Self>) -> Task<()> {
        Task::ready(())
    }

    /// Watches the repository at `root` and takes in what changes.
    #[cfg(not(test))]
    fn watch(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        let dirs = cx.background_spawn({
            let root = root.clone();
            async move { git::git_dirs(&root) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let dirs = dirs.await.unwrap_or_else(|error| {
                log::warn!("git folder unknown, watching the working tree only: {error}");
                Vec::new()
            });
            let (watcher, batches) = match watcher::RepositoryWatcher::new(&root, &dirs) {
                Ok(watching) => watching,
                Err(error) => {
                    log::error!("cannot watch {}: {error}", root.display());
                    return;
                }
            };
            while let Ok(batch) = batches.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.files_changed(&batch, window, cx))
                    .is_err()
                {
                    break;
                }
            }
            drop(watcher);
        })
    }

    /// Follows changes made on disk, by anyone: reads the changes again
    /// unless only ignored paths changed, and checks the open files that
    /// changed (design 5).
    fn files_changed(&mut self, batch: &Batch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let effects = watcher::classify(batch, |path| {
            repository
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.is_ignored(path))
        });
        let changed_buffers: Vec<Entity<FileBuffer>> = self
            .editors
            .buffers
            .iter()
            .filter(|(path, _)| {
                effects.everything
                    || repository
                        .relative(path)
                        .is_some_and(|relative| effects.files.contains(&relative))
            })
            .map(|(_, open)| open.buffer.clone())
            .collect();
        for buffer in changed_buffers {
            buffer.update(cx, |buffer, cx| buffer.disk_changed(window, cx));
        }
        // Folders on screen are read again; ones read before but folded away
        // are forgotten, to be read when expanded.
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        let changed: Vec<String> = if effects.everything {
            repository.explorer.shown_folders().map(str::to_owned).collect()
        } else {
            effects.folders.iter().cloned().collect()
        };
        let mut reread = Vec::new();
        for folder in changed {
            let shown = folder.is_empty() || repository.explorer.is_expanded(&folder);
            if shown {
                reread.push(folder);
            } else {
                repository.explorer.forget(&folder);
            }
        }
        if !reread.is_empty() {
            self.read_folders(reread, None, cx);
        }
        if effects.refresh {
            self.refresh(window, cx);
        }
    }

    /// Runs `then`, which drops the open repository, once the unsaved edits
    /// of its files are saved or given up; nothing happens when the user
    /// cancels. Files outside it keep their tabs and their edits.
    fn after_unsaved_files(
        &mut self,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = match self.repository.as_ref() {
            Some(repository) => self.dirty_files(Some(&repository.root), cx),
            None => Vec::new(),
        };
        let confirmed = self.confirm_unsaved(dirty, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if !confirmed.await {
                return;
            }
            if let Err(error) = this.update_in(cx, |this, window, cx| then(this, window, cx)) {
                log::warn!("view released before leaving the repository: {error:#}");
            }
        })
        .detach();
    }

    /// Closes the open repository's tabs: its diffs and its files. The
    /// tabs of other files and the settings stay.
    fn leave_repository(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.take() else {
            return;
        };
        let gone: Vec<TabKey> = self
            .editors
            .group
            .tabs()
            .iter()
            .filter(|key| match key {
                TabKey::Diff { .. } => true,
                TabKey::File(path) => path.starts_with(&repository.root),
                TabKey::Settings => false,
            })
            .cloned()
            .collect();
        for key in &gone {
            self.editors.close(key);
        }
        self.show_active_tab(cx);
    }

    fn show_repository(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_repository(cx);
        window.set_window_title(&folder_name(&root));
        self.recent.record(&root);
        let diff = cx.new(|cx| {
            DiffState::new(Vec::new(), cx)
                .with_mode(DiffMode::Split)
                .with_context_lines(None)
        });
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .placeholder(commit_placeholder(None))
        });
        let diff_observation = cx.observe(&diff, |_, _, cx| cx.notify());
        let watching = Self::watch(root.clone(), window, cx);
        self.repository = Some(Repository {
            root,
            side_view: SideView::SourceControl,
            explorer: Explorer::default(),
            explorer_scroll: UniformListScrollHandle::new(),
            explorer_collapsed: false,
            decorations: Decorations::default(),
            diff,
            shown_patch: None,
            message,
            load_state: LoadState::Loading,
            snapshot: None,
            collapsed_groups: Vec::new(),
            operation: None,
            refresh_task: Task::ready(()),
            _watching: watching,
            _diff_observation: diff_observation,
        });
        self.open_error = None;
        // Files that were outside the old repository may be inside this one.
        self.follow_outside_files(window, cx);
        self.refresh(window, cx);
        self.read_folders(vec![String::new()], None, cx);
    }

    fn show_open_error(
        &mut self,
        message: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.set_window_title(APP_NAME);
        self.leave_repository(cx);
        self.follow_outside_files(window, cx);
        self.open_error = Some(message);
        cx.notify();
    }

    fn close_folder(&mut self, _: &CloseFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.repository.is_none() {
            return;
        }
        self.after_unsaved_files(
            |this, window, cx| {
                this.leave_repository(cx);
                this.follow_outside_files(window, cx);
                window.set_window_title(APP_NAME);
                this.open_error = None;
                cx.notify();
            },
            window,
            cx,
        );
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        let root = repository.root.clone();
        let shown_key = repository
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.key.to_string());
        let load = cx.background_spawn(async move {
            git::load_snapshot(&root, shown_key.as_deref())
                .map_err(|error| SharedString::from(error.to_string()))
        });
        repository.refresh_task = cx.spawn_in(window, async move |this, cx| {
            let snapshot = load.await;
            let applied = this.update_in(cx, |this, window, cx| {
                this.apply_snapshot(snapshot, window, cx)
            });
            if let Err(error) = applied {
                log::warn!("view released before refresh finished: {error:#}");
            }
        });
    }

    fn apply_snapshot(
        &mut self,
        snapshot: Result<Option<Snapshot>, SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        match snapshot {
            Ok(None) => repository.load_state = LoadState::Loaded,
            Ok(Some(snapshot)) => {
                let first = repository.snapshot.is_none();
                let placeholder_changed =
                    repository.branch().map(|branch| &branch.head) != Some(&snapshot.branch.head);
                // VS Code fills the commit field with git's merge message
                // once, when a merge stops; never over what was typed.
                let merge_started = snapshot.in_progress == Some(git::InProgress::Merge)
                    && repository
                        .snapshot
                        .as_ref()
                        .and_then(|shown| shown.in_progress)
                        != Some(git::InProgress::Merge);
                if merge_started
                    && let Some(message) = snapshot.merge_message.clone()
                    && repository.message.read(cx).value().is_empty()
                {
                    repository
                        .message
                        .update(cx, |state, cx| state.set_value(message, window, cx));
                }
                repository.load_state = LoadState::Loaded;
                repository.decorations = Decorations::of(&snapshot);
                repository.snapshot = Some(snapshot);
                if placeholder_changed {
                    let placeholder = commit_placeholder(repository.branch());
                    repository.message.update(cx, |state, cx| {
                        state.set_placeholder(placeholder, window, cx)
                    });
                }
                // A diff tab whose file left its group has nothing to show.
                let stale: Vec<TabKey> = self
                    .editors
                    .group
                    .tabs()
                    .iter()
                    .filter(|key| match key {
                        TabKey::Diff { group, path } => repository.file(*group, path).is_none(),
                        TabKey::File(_) | TabKey::Settings => false,
                    })
                    .cloned()
                    .collect();
                for key in &stale {
                    self.editors.close(key);
                }
                // The first file opens with the repository, and again when the
                // closing above left no tab; not after the user closed them.
                let emptied = !stale.is_empty() && self.editors.group.tabs().is_empty();
                if (first || emptied)
                    && let Some(changed) = repository.files().first()
                {
                    let key = TabKey::Diff {
                        group: changed.group,
                        path: changed.file.path().clone(),
                    };
                    self.editors.group.open(key, true);
                }
                self.show_active_tab(cx);
                // The tabs changed under the user: show the new active file.
                if first || !stale.is_empty() {
                    self.reveal_active_file(cx);
                }
            }
            Err(message) => repository.load_state = LoadState::Failed(message),
        }
        cx.notify();
    }

    /// Shows the active tab's content and scrolls its tab into view.
    fn show_active_tab(&mut self, cx: &mut App) {
        if let Some(repository) = self.repository.as_mut() {
            repository.show_active_diff(self.editors.group.active(), cx);
        }
        self.editors.scroll_to_active();
    }

    /// The changed file of the active tab, when that is a diff tab.
    fn active_diff(&self) -> Option<&ChangedFile> {
        self.repository.as_ref()?.diff_of(self.editors.group.active())
    }

    /// The file tab of `path`, relative to the open repository's root.
    fn file_key(&self, path: &str) -> Option<TabKey> {
        let repository = self.repository.as_ref()?;
        Some(TabKey::File(repository.root.join(path).into()))
    }

    /// 「打开文件」 of a change: its file tab, `path` relative to the root,
    /// opened for good to edit what is on disk.
    fn open_changed_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.file_key(path) {
            self.open_tab(key, false, window, cx)
        }
    }

    /// Opens `key` in a tab, as a preview tab or for good, and shows it.
    fn open_tab(&mut self, key: TabKey, preview: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let TabKey::File(path) = &key
            && !self.editors.buffers.contains_key(path)
        {
            let buffer = FileBuffer::open(path.clone(), window, cx);
            let pinned = key.clone();
            let subscriptions = [
                cx.observe(&buffer, |_, _, cx| cx.notify()),
                // VS Code keeps a preview tab once its file has unsaved edits.
                cx.subscribe(&buffer, move |this, _, event, cx| match event {
                    FileBufferEvent::Dirtied => {
                        this.editors.group.pin(&pinned);
                        cx.notify();
                    }
                }),
            ];
            self.editors.buffers.insert(
                path.clone(),
                OpenBuffer {
                    buffer,
                    _subscriptions: subscriptions,
                },
            );
        }
        // Opened for good, a file takes the keyboard, as in VS Code; a
        // preview leaves it in the list it was opened from.
        if !preview && let TabKey::File(path) = &key {
            let buffer = self.editors.buffers[path].buffer.clone();
            buffer.update(cx, |buffer, cx| buffer.focus(window, cx));
        }
        if let Opened::Replaced(replaced) = self.editors.group.open(key, preview) {
            self.editors.tab_gone(&replaced);
        }
        self.show_active_tab(cx);
        self.reveal_active_file(cx);
        self.follow_outside_files(window, cx);
        cx.notify();
    }

    fn activate_tab(&mut self, key: &TabKey, window: &mut Window, cx: &mut Context<Self>) {
        self.editors.group.activate(key);
        self.show_active_tab(cx);
        self.focus_active_tab(window, cx);
        self.reveal_active_file(cx);
        cx.notify();
    }

    /// Hands the keyboard to the active tab's content, as VS Code does when
    /// a tab is picked or takes over: a file's editor, the settings page, or
    /// the window for a diff, which takes no keyboard. Left where it was,
    /// it would stay with content no longer drawn, out of every key binding.
    fn focus_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.editors.group.active() {
            Some(TabKey::File(_)) => {
                if let Some(buffer) = self.editors.active_buffer().cloned() {
                    buffer.update(cx, |buffer, cx| buffer.focus(window, cx));
                }
            }
            Some(TabKey::Settings) => {
                if let Some(open) = &self.editors.settings {
                    let focus = open.editor.focus_handle(cx);
                    window.focus(&focus, cx);
                }
            }
            Some(TabKey::Diff { .. }) | None => window.focus(&self.focus_handle, cx),
        }
    }

    fn pin_tab(&mut self, key: &TabKey, cx: &mut Context<Self>) {
        self.editors.group.pin(key);
        cx.notify();
    }

    /// ⌘S: writes the active file tab back to its file.
    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(TabKey::File(path)) = self.editors.group.active().cloned() {
            self.save_file(path, false, window, cx).detach();
        }
    }

    /// Saves the file tab of `path`, reporting a failure; resolves to whether
    /// the file was written.
    fn save_file(
        &mut self,
        path: Arc<Path>,
        overwrite: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let Some(buffer) = self
            .editors
            .buffers
            .get(&path)
            .map(|open| open.buffer.clone())
        else {
            return Task::ready(false);
        };
        let saving = buffer.update(cx, |buffer, cx| buffer.save(overwrite, cx));
        cx.spawn_in(window, async move |this, cx| match saving.await {
            Ok(()) => true,
            Err(error) => {
                let reported = this.update_in(cx, |this, window, cx| {
                    this.report_save_error(path, error, window, cx)
                });
                if let Err(error) = reported {
                    log::warn!("view released before a failed save was reported: {error:#}");
                }
                false
            }
        })
    }

    /// VS Code's notice for a failed save; a file changed on disk since it
    /// was read offers to overwrite it.
    fn report_save_error(
        &mut self,
        path: Arc<Path>,
        error: SaveError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = file_name(&path);
        let notification = match error {
            SaveError::Conflict => {
                let workbench = cx.entity().downgrade();
                Notification::error(format!(
                    "无法保存“{name}”：文件内容较新。请用你的更改覆盖文件内容，或放弃保存。"
                ))
                .action(move |_, _, cx| {
                    let workbench = workbench.clone();
                    let path = path.clone();
                    let notification = cx.entity().downgrade();
                    Button::new("save-overwrite")
                        .label("覆盖")
                        .on_click(move |_, window, cx| {
                            if let Some(notification) = notification.upgrade() {
                                notification.update(cx, |notification, cx| {
                                    notification.dismiss(window, cx)
                                });
                            }
                            if let Some(workbench) = workbench.upgrade() {
                                workbench.update(cx, |workbench, cx| {
                                    workbench.save_file(path.clone(), true, window, cx).detach()
                                });
                            }
                        })
                })
            }
            SaveError::Io(reason) => {
                Notification::error(format!("无法保存“{name}”：{reason}")).autohide(false)
            }
        };
        window.push_notification(notification, cx);
    }

    /// ⌘Q: quits once the unsaved edits are saved or given up. Without a
    /// window the app-wide handler quits at once.
    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        let confirmed = self.confirm_unsaved_files(window, cx);
        cx.spawn(async move |_, cx| {
            if confirmed.await {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    }

    /// ⌘W: closes the active tab.
    fn close_active_tab(&mut self, _: &CloseActiveTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = self.editors.group.active().cloned() {
            self.request_close_tab(active, window, cx);
        }
    }

    /// Closes `key` at the user's request, first asking what to do with
    /// unsaved edits, as VS Code does.
    fn request_close_tab(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = match &key {
            TabKey::File(path) => self
                .editors
                .buffers
                .get(path)
                .is_some_and(|open| open.buffer.read(cx).is_dirty(cx))
                .then(|| path.clone()),
            TabKey::Diff { .. } | TabKey::Settings => None,
        };
        let Some(path) = dirty else {
            self.close_tab(&key, window, cx);
            return;
        };
        let confirmed = self.confirm_unsaved(vec![path], window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if confirmed.await
                && let Err(error) =
                    this.update_in(cx, |this, window, cx| this.close_tab(&key, window, cx))
            {
                log::warn!("view released before closing a tab: {error:#}");
            }
        })
        .detach();
    }

    /// The files of the file tabs with unsaved edits, in tab order; only
    /// those inside `within` when given.
    fn dirty_files(&self, within: Option<&Path>, cx: &App) -> Vec<Arc<Path>> {
        self.editors
            .group
            .tabs()
            .iter()
            .filter_map(|key| match key {
                TabKey::File(path) => Some(path),
                TabKey::Diff { .. } | TabKey::Settings => None,
            })
            .filter(|path| within.is_none_or(|within| path.starts_with(within)))
            .filter(|path| {
                self.editors
                    .buffers
                    .get(*path)
                    .is_some_and(|open| open.buffer.read(cx).is_dirty(cx))
            })
            .cloned()
            .collect()
    }

    /// Asks what to do with the unsaved edits of `paths` before they are
    /// dropped, with VS Code's dialogs: one file asks to save it, several
    /// list them and offer to save all. Resolves to whether to go on: saved
    /// (every save succeeded) or not saved, but not canceled.
    fn confirm_unsaved(
        &mut self,
        paths: Vec<Arc<Path>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let answer = match paths.as_slice() {
            [] => return Task::ready(true),
            [path] => window.prompt(
                PromptLevel::Warning,
                &format!("是否要保存对“{}”的更改？", file_name(path)),
                Some("如果不保存，你的更改将丢失。"),
                &["保存", "不保存", "取消"],
                cx,
            ),
            paths => {
                let names: Vec<String> = paths.iter().map(|path| file_name(path)).collect();
                window.prompt(
                    PromptLevel::Warning,
                    &format!("是否要保存对下列 {} 个文件的更改？", paths.len()),
                    Some(&format!("{}\n\n如果不保存，你的更改将丢失。", names.join("\n"))),
                    &["全部保存", "不保存", "取消"],
                    cx,
                )
            }
        };
        cx.spawn_in(window, async move |this, cx| match answer.await {
            Ok(0) => {
                let saves = this.update_in(cx, |this, window, cx| {
                    paths
                        .into_iter()
                        .map(|path| this.save_file(path, false, window, cx))
                        .collect::<Vec<_>>()
                });
                let Ok(saves) = saves else {
                    return false;
                };
                let mut saved = true;
                for save in saves {
                    saved &= save.await;
                }
                saved
            }
            Ok(1) => true,
            _ => false,
        })
    }

    /// Asks about every file tab with unsaved edits before they all go:
    /// quitting, closing the window.
    fn confirm_unsaved_files(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let dirty = self.dirty_files(None, cx);
        self.confirm_unsaved(dirty, window, cx)
    }

    /// Closes `key`. Closing the active tab hands the keyboard to the tab
    /// that takes over.
    fn close_tab(&mut self, key: &TabKey, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = self.editors.group.active() == Some(key);
        self.editors.close(key);
        self.show_active_tab(cx);
        if was_active {
            self.focus_active_tab(window, cx);
            self.reveal_active_file(cx);
        }
        self.follow_outside_files(window, cx);
        cx.notify();
    }

    fn toggle_group(&mut self, group: ChangeGroup, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        if let Some(ix) = repository.collapsed_groups.iter().position(|g| *g == group) {
            repository.collapsed_groups.remove(ix);
        } else {
            repository.collapsed_groups.push(group);
        }
        cx.notify();
    }

    /// Runs `work` in the background as the one repository change in
    /// progress and refreshes once it is done. The result is handed back
    /// rather than reported, for flows that answer some failures themselves.
    /// `None` when another change is still running or nothing is open.
    fn start_operation(
        &mut self,
        operation: Operation,
        work: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<oneshot::Receiver<Result<(), GitError>>> {
        let repository = self.repository.as_mut()?;
        if repository.is_busy() {
            return None;
        }
        let root = repository.root.clone();
        let (done, result) = oneshot::channel();
        let task = cx.spawn_in(window, async move |this, cx| {
            let outcome = cx.background_spawn(async move { work(&root) }).await;
            let finished = this.update_in(cx, |this, window, cx| {
                if let Some(repository) = this.repository.as_mut() {
                    repository.operation = None;
                }
                this.refresh(window, cx);
                cx.notify();
            });
            if let Err(error) = finished {
                log::warn!("view released during {operation:?}: {error:#}");
            }
            if done.send(outcome).is_err() {
                log::warn!("nobody awaited the result of {operation:?}");
            }
        });
        repository.operation = Some(RunningOperation {
            operation,
            _task: task,
        });
        cx.notify();
        Some(result)
    }

    /// Runs `work` like [`Self::start_operation`], then calls `then` on
    /// success or reports the failure.
    fn run_operation(
        &mut self,
        operation: Operation,
        work: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = self.start_operation(operation, work, window, cx) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            // A closed repository drops the work and its sender with it.
            let Ok(result) = result.await else {
                return;
            };
            let finished = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => then(this, window, cx),
                Err(error) => report_failure(operation, &error, window, cx),
            });
            if let Err(error) = finished {
                log::warn!("view released after {operation:?}: {error:#}");
            }
        })
        .detach();
    }

    fn stage(&mut self, files: Vec<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<String> = files.into_iter().flatten().collect();
        // Without paths, `git add --all` would take the whole tree.
        if paths.is_empty() {
            return;
        }
        self.run_operation(
            Operation::Stage,
            move |root| git::stage(root, &paths),
            |_, _, _| {},
            window,
            cx,
        );
    }

    fn unstage(&mut self, files: Vec<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(has_commits) = self
            .repository
            .as_ref()
            .and_then(|repository| repository.branch())
            .map(|branch| branch.has_commits)
        else {
            return;
        };
        let paths: Vec<String> = files.into_iter().flatten().collect();
        if paths.is_empty() {
            return;
        }
        self.run_operation(
            Operation::Unstage,
            move |root| git::unstage(root, &paths, has_commits),
            |_, _, _| {},
            window,
            cx,
        );
    }

    /// Asks before discarding: tracked files lose their unstaged changes,
    /// untracked files are deleted for good.
    fn discard(&mut self, files: Vec<ChangedFile>, window: &mut Window, cx: &mut Context<Self>) {
        if files.is_empty() || self.repository.as_ref().is_none_or(Repository::is_busy) {
            return;
        }
        let (untracked, tracked): (Vec<_>, Vec<_>) = files
            .iter()
            .partition(|changed| changed.letter == git::StatusLetter::Untracked);
        let (message, detail, confirm) = match files.as_slice() {
            [only] if tracked.is_empty() => (
                format!("确定要删除 {} 吗？", only.file.path()),
                Some("此操作不可撤销！如果继续，该文件将永久丢失。".to_owned()),
                "删除文件",
            ),
            [only] => (
                format!("确定要放弃 {} 中的更改吗？", only.file.path()),
                None,
                "放弃文件",
            ),
            _ => (
                format!("确定要放弃全部 {} 个文件中的更改吗？", files.len()),
                (!untracked.is_empty()).then(|| {
                    format!(
                        "其中 {} 个未跟踪文件将被永久删除，此操作不可撤销。",
                        untracked.len()
                    )
                }),
                "放弃所有文件",
            ),
        };
        let tracked: Vec<String> = tracked
            .iter()
            .flat_map(|changed| paths_of(changed))
            .collect();
        let untracked: Vec<String> = untracked
            .iter()
            .map(|changed| changed.file.path().to_string())
            .collect();
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            detail.as_deref(),
            &[confirm, "取消"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let started = this.update_in(cx, |this, window, cx| {
                this.run_operation(
                    Operation::Discard,
                    move |root| git::discard(root, &tracked, &untracked),
                    |_, _, _| {},
                    window,
                    cx,
                )
            });
            if let Err(error) = started {
                log::warn!("view released before discarding: {error:#}");
            }
        })
        .detach();
    }

    /// The files of a group, as the group header's buttons act on them.
    fn group_files(&self, group: ChangeGroup) -> Vec<ChangedFile> {
        self.repository
            .as_ref()
            .map(|repository| repository.group_files(group).cloned().collect())
            .unwrap_or_default()
    }

    fn group_paths(&self, group: ChangeGroup) -> Vec<Vec<String>> {
        self.group_files(group).iter().map(paths_of).collect()
    }

    fn commit(
        &mut self,
        amend: bool,
        after: AfterCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        if repository.is_busy() {
            return;
        }
        let message = repository.message.read(cx).value().to_string();
        if amend {
            self.start_commit(message, true, false, after, window, cx);
            return;
        }
        if message.trim().is_empty() {
            window.push_notification(Notification::warning("请输入提交信息"), cx);
            return;
        }
        let has_staged = repository.group_files(ChangeGroup::Staged).next().is_some();
        if has_staged {
            self.start_commit(message, false, false, after, window, cx);
            return;
        }
        if repository
            .group_files(ChangeGroup::Changes)
            .next()
            .is_none()
        {
            window.push_notification(Notification::info("没有可提交的更改"), cx);
            return;
        }
        // Nothing staged: VS Code commits everything, asks first, or, told
        // never to ask, commits nothing.
        let settings = *SettingsStore::global(cx).values();
        if settings.enable_smart_commit {
            self.start_commit(message, false, true, after, window, cx);
            return;
        }
        if !settings.suggest_smart_commit {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Info,
            "没有暂存的更改，是否暂存全部更改并直接提交？",
            None,
            &["是", "始终", "从不", "取消"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let started = this.update_in(cx, |this, window, cx| match answer {
                Ok(0) => this.start_commit(message, false, true, after, window, cx),
                Ok(1) => {
                    // Remembered or not, this commit goes ahead.
                    this.write_setting("git.enableSmartCommit", true.into(), window, cx);
                    this.start_commit(message, false, true, after, window, cx)
                }
                Ok(2) => {
                    this.write_setting("git.suggestSmartCommit", false.into(), window, cx);
                    // Unstaged changes no longer count: the button disables.
                    cx.notify();
                }
                _ => {}
            });
            if let Err(error) = started {
                log::warn!("view released before committing: {error:#}");
            }
        })
        .detach();
    }

    fn start_commit(
        &mut self,
        message: String,
        amend: bool,
        stage_everything: bool,
        after: AfterCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_operation(
            Operation::Commit,
            move |root| {
                if stage_everything {
                    git::stage_all(root)?;
                }
                git::commit(root, &message, amend)
            },
            move |this, window, cx| {
                if let Some(repository) = this.repository.as_ref() {
                    repository
                        .message
                        .update(cx, |state, cx| state.set_value("", window, cx));
                }
                match after {
                    AfterCommit::Nothing => {}
                    AfterCommit::Push => this.push(window, cx),
                    AfterCommit::Sync => this.sync(window, cx),
                }
            },
            window,
            cx,
        );
    }

    /// Pushes to the upstream, or publishes a branch that has none.
    fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.repository.as_ref().and_then(Repository::branch) else {
            return;
        };
        if branch.upstream.is_some() {
            self.run_operation(Operation::Push, git::push, |_, _, _| {}, window, cx);
            return;
        }
        match branch.head {
            Head::Branch(_) => self.publish(window, cx),
            Head::Detached(_) => window.push_notification(
                Notification::warning("当前处于分离头指针状态，没有可推送的分支"),
                cx,
            ),
        }
    }

    /// Pulls then pushes after asking, as VS Code does; a branch without an
    /// upstream is published instead.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.repository.as_ref().and_then(Repository::branch) else {
            return;
        };
        let Some(upstream) = branch.upstream.clone() else {
            self.push(window, cx);
            return;
        };
        if !SettingsStore::global(cx).values().confirm_sync {
            self.start_sync(window, cx);
            return;
        }
        let answer = window.prompt(
            PromptLevel::Info,
            &format!(
                "此操作将从“{}”拉取提交，并将本地提交推送到该分支。",
                upstream.name
            ),
            None,
            &["确定", "确定，且不再显示", "取消"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let started = this.update_in(cx, |this, window, cx| match answer {
                Ok(0) => this.start_sync(window, cx),
                Ok(1) => {
                    this.write_setting("git.confirmSync", false.into(), window, cx);
                    this.start_sync(window, cx)
                }
                _ => {}
            });
            if let Err(error) = started {
                log::warn!("view released before syncing: {error:#}");
            }
        })
        .detach();
    }

    fn start_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_operation(
            Operation::Sync,
            |root| {
                git::pull(root)?;
                git::push(root)
            },
            |_, _, _| {},
            window,
            cx,
        );
    }

    /// Opens the repository root in the terminal; does nothing on the
    /// welcome page, where there is no folder to open.
    fn open_terminal(&mut self, _: &OpenTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let root = repository.root.clone();
        let opened = cx.background_spawn(async move { terminal::open(&root) });
        cx.spawn_in(window, async move |this, cx| {
            let Err(error) = opened.await else {
                return;
            };
            let reported = this.update_in(cx, |_, window, cx| {
                window.push_notification(
                    Notification::error(error)
                        .title("无法打开终端")
                        .autohide(false),
                    cx,
                )
            });
            if let Err(error) = reported {
                log::warn!("view released before reporting the terminal failure: {error:#}");
            }
        })
        .detach();
    }

    fn about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Info,
            APP_NAME,
            Some(concat!("版本 ", env!("CARGO_PKG_VERSION"))),
            &["好"],
            cx,
        );
        cx.background_spawn(async move {
            if answer.await.is_err() {
                log::warn!("about prompt dismissed without an answer");
            }
        })
        .detach();
    }

    fn render_repository(
        &self,
        repository: &Repository,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(self.render_activity_bar(repository, cx))
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            h_resizable("workbench-panels")
                                // One panel for either view, so switching
                                // keeps the width.
                                .child(resizable_panel().size(px(300.)).child(
                                    self.render_side_bar(repository, window, cx),
                                ))
                                .child(
                                    resizable_panel().child(self.render_editor(cx)),
                                ),
                        ),
                    ),
            )
            .child(self.render_status_bar(repository, cx))
    }
}

impl Workbench {
    /// The side bar: the chosen view, above the activity bar's row when
    /// that is at the bottom.
    fn render_side_bar(
        &self,
        repository: &Repository,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (sidebar, sidebar_border) = {
            let theme = cx.theme();
            (theme.sidebar, theme.sidebar_border)
        };
        let view = match repository.side_view {
            SideView::Explorer => self
                .render_explorer(repository, window, cx)
                .into_any_element(),
            SideView::SourceControl => self
                .render_scm_panel(repository, window, cx)
                .into_any_element(),
        };
        let footer = self.render_activity_footer(repository, cx);
        v_flex()
            .size_full()
            .child(div().flex_1().min_h_0().child(view))
            .when_some(footer, |this, footer| {
                this.child(
                    div()
                        .bg(sidebar)
                        .border_r_1()
                        .border_color(sidebar_border)
                        .child(footer),
                )
            })
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(13.))
            .on_action(
                cx.listener(|this, _: &OpenFolder, window, cx| this.prompt_open_folder(window, cx)),
            )
            .on_action(cx.listener(Self::close_folder))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::close_active_tab))
            .on_action(cx.listener(Self::open_user_settings_json))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::show_explorer))
            .on_action(cx.listener(Self::show_source_control))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(Self::open_terminal))
            .on_action(cx.listener(Self::about))
            .on_action(cx.listener(|this, _: &Commit, window, cx| {
                this.commit(false, AfterCommit::Nothing, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CommitAmend, window, cx| {
                this.commit(true, AfterCommit::Nothing, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CommitAndPush, window, cx| {
                this.commit(false, AfterCommit::Push, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CommitAndSync, window, cx| {
                this.commit(false, AfterCommit::Sync, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Pull, window, cx| {
                this.run_operation(Operation::Pull, git::pull, |_, _, _| {}, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Push, window, cx| this.push(window, cx)))
            .on_action(cx.listener(|this, _: &Sync, window, cx| this.sync(window, cx)))
            .on_action(cx.listener(|this, _: &Fetch, window, cx| {
                this.run_operation(Operation::Fetch, git::fetch, |_, _, _| {}, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Checkout, window, cx| this.checkout(window, cx)))
            .on_action(cx.listener(|this, _: &CreateBranch, window, cx| {
                this.create_branch(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CreateBranchFrom, window, cx| {
                this.create_branch(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &RenameBranch, window, cx| this.rename_branch(window, cx)),
            )
            .on_action(cx.listener(|this, _: &DeleteBranch, window, cx| {
                this.delete_branch(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteRemoteBranch, window, cx| {
                this.delete_branch(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &MergeBranch, window, cx| this.merge_branch(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &RebaseBranch, window, cx| this.rebase_branch(window, cx)),
            )
            .on_action(cx.listener(|this, _: &PublishBranch, window, cx| this.publish(window, cx)))
            .on_action(cx.listener(|this, _: &AbortMerge, window, cx| {
                this.abort_in_progress(git::InProgress::Merge, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AbortRebase, window, cx| {
                this.abort_in_progress(git::InProgress::Rebase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &StageAll, window, cx| {
                this.stage(this.group_paths(ChangeGroup::Changes), window, cx)
            }))
            .on_action(cx.listener(|this, _: &UnstageAll, window, cx| {
                this.unstage(this.group_paths(ChangeGroup::Staged), window, cx)
            }))
            .on_action(cx.listener(|this, _: &DiscardAll, window, cx| {
                let files = this.group_files(ChangeGroup::Changes);
                this.discard(files, window, cx)
            }))
            .map(|this| match self.repository.as_ref() {
                Some(repository) => this.child(self.render_repository(repository, window, cx)),
                None => this.child(self.render_editor(cx)),
            })
            .when_some(self.quick_input.as_ref(), |this, (quick_input, _)| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(quick_input.view()),
                )
            })
    }
}

fn report_failure(operation: Operation, error: &GitError, window: &mut Window, cx: &mut App) {
    let message = if error.needs_authentication() {
        format!("需要认证，请在终端中完成 git 认证配置后重试。\n\n{error}")
    } else {
        error.to_string()
    };
    window.push_notification(
        Notification::error(message)
            .title(format!("{}失败", operation.label()))
            .autohide(false),
        cx,
    );
}

/// VS Code's commit message placeholder, naming the branch to commit on.
fn commit_placeholder(branch: Option<&BranchStatus>) -> String {
    match branch.map(|branch| &branch.head) {
        Some(Head::Branch(name)) => format!("消息(⌘Enter 在\"{name}\"提交)"),
        _ => "消息(⌘Enter 提交)".to_owned(),
    }
}

/// The last part of `path`, as dialogs and notices name a file.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}
