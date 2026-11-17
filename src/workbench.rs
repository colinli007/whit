//! The window: the welcome page until a repository is open, then VS Code's
//! Source Control layout — the side bar, the diff editor and the status bar.
//! Each area renders from its own file; they share this one state.
mod branches;
mod editor;
mod quick_input;
mod relative_time;
mod scm_panel;
mod status_bar;
mod welcome;

#[cfg(test)]
mod tests;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_channel::oneshot;
use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    diff::{DiffMode, DiffState},
    input::TextareaState,
    notification::Notification,
    resizable::{h_resizable, resizable_panel},
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::{
    APP_NAME, AbortMerge, AbortRebase, About, Checkout, CloseFolder, Commit, CommitAmend,
    CommitAndPush, CommitAndSync, CreateBranch, CreateBranchFrom, DeleteBranch, DeleteRemoteBranch,
    DiscardAll, Fetch, MergeBranch, OpenFolder, OpenTerminal, PublishBranch, Pull, Push,
    RebaseBranch, Refresh, RenameBranch, StageAll, Sync, UnstageAll,
    git::{self, BranchStatus, ChangeGroup, ChangedFile, GitError, Head, Snapshot},
    recent::RecentFolders,
    settings::Settings,
    terminal,
};

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
    recent: RecentFolders,
    settings: Settings,
    /// Whether the welcome page lists every recent folder or only the first ten.
    show_all_recent: bool,
    repository: Option<Repository>,
    /// Why the last folder could not be opened, shown on the welcome page.
    open_error: Option<SharedString>,
    opening: Task<()>,
    /// Set when the window loses focus. Only a return after leaving can mean
    /// the files changed elsewhere; the activation that follows opening the
    /// window would just repeat the load the opening already started.
    left_window: bool,
    /// The picker or input box dropped from the top, with its dismissal
    /// subscription.
    quick_input: Option<(quick_input::QuickInput, Subscription)>,
    _activation: Subscription,
}

struct Repository {
    root: PathBuf,
    /// Holds only the open file: the editor shows one file at a time.
    diff: Entity<DiffState>,
    message: Entity<TextareaState>,
    load_state: LoadState,
    snapshot: Option<Snapshot>,
    /// The file in the editor; a file may be in two groups at once.
    open: Option<OpenFile>,
    collapsed_groups: Vec<ChangeGroup>,
    /// The repository change in progress; only one runs at a time.
    operation: Option<RunningOperation>,
    /// Replacing this task cancels a refresh still in flight, so only the
    /// latest refresh can ever update the view.
    refresh_task: Task<()>,
    _diff_observation: Subscription,
}

#[derive(Clone, PartialEq, Eq)]
struct OpenFile {
    group: ChangeGroup,
    path: SharedString,
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

    fn file(&self, open: &OpenFile) -> Option<&ChangedFile> {
        self.files()
            .iter()
            .find(|changed| changed.group == open.group && changed.file.path() == &open.path)
    }

    fn open_file(&self) -> Option<&ChangedFile> {
        self.file(self.open.as_ref()?)
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

    /// Puts the open file in the diff unless `shown`, the patch on screen,
    /// already is its patch: replacing the file resets the scroll position,
    /// expanded folds and line selection.
    fn show_open(&self, shown: Option<Arc<str>>, cx: &mut App) {
        let open = self.open_file();
        if open.map(|changed| &changed.patch) == shown.as_ref() {
            return;
        }
        let files: Vec<_> = open
            .map(|changed| changed.file.clone())
            .into_iter()
            .collect();
        self.diff.update(cx, |diff, cx| diff.set_files(files, cx));
    }

    fn shown_patch(&self) -> Option<Arc<str>> {
        self.open_file().map(|changed| changed.patch.clone())
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
        settings: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        // The typical flow is switching to a terminal where a CLI edits files and
        // switching back, so regaining focus is exactly when the diff goes stale.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.left_window = true;
                this.dismiss_picker_on_blur(window, cx);
            } else if this.left_window {
                this.left_window = false;
                this.refresh(window, cx);
            }
        });
        let mut view = Self {
            focus_handle,
            scm_list_focus: cx.focus_handle(),
            scm_hovered_row: None,
            recent,
            settings,
            show_all_recent: false,
            repository: None,
            open_error: None,
            opening: Task::ready(()),
            left_window: false,
            quick_input: None,
            _activation: activation,
        };
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
                Ok(root) => this.show_repository(root, window, cx),
                Err(error) => this.show_open_error(error.to_string().into(), window, cx),
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
                Ok(root) => this.show_repository(root, window, cx),
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

    fn show_repository(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
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
        self.repository = Some(Repository {
            root,
            diff,
            message,
            load_state: LoadState::Loading,
            snapshot: None,
            open: None,
            collapsed_groups: Vec::new(),
            operation: None,
            refresh_task: Task::ready(()),
            _diff_observation: diff_observation,
        });
        self.open_error = None;
        self.refresh(window, cx);
    }

    fn show_open_error(
        &mut self,
        message: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.set_window_title(APP_NAME);
        self.repository = None;
        self.open_error = Some(message);
        cx.notify();
    }

    fn close_folder(&mut self, _: &CloseFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.repository.take().is_none() {
            return;
        }
        window.set_window_title(APP_NAME);
        self.open_error = None;
        cx.notify();
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
                let shown = repository.shown_patch();
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
                repository.snapshot = Some(snapshot);
                if repository.open_file().is_none() {
                    repository.open = repository.files().first().map(|changed| OpenFile {
                        group: changed.group,
                        path: changed.file.path().clone(),
                    });
                }
                repository.show_open(shown, cx);
                if placeholder_changed {
                    let placeholder = commit_placeholder(repository.branch());
                    repository.message.update(cx, |state, cx| {
                        state.set_placeholder(placeholder, window, cx)
                    });
                }
            }
            Err(message) => repository.load_state = LoadState::Failed(message),
        }
        cx.notify();
    }

    fn open_change(&mut self, group: ChangeGroup, path: SharedString, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        let shown = repository.shown_patch();
        repository.open = Some(OpenFile { group, path });
        repository.show_open(shown, cx);
        cx.notify();
    }

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_mut() else {
            return;
        };
        let shown = repository.shown_patch();
        repository.open = None;
        repository.show_open(shown, cx);
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

    fn open_with_system(&mut self, path: SharedString, cx: &mut Context<Self>) {
        if let Some(repository) = self.repository.as_ref() {
            cx.open_with_system(&repository.root.join(path.as_str()));
        }
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
        let settings = *self.settings.values();
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
                    this.settings
                        .update(|settings| settings.enable_smart_commit = true);
                    this.start_commit(message, false, true, after, window, cx)
                }
                Ok(2) => {
                    this.settings
                        .update(|settings| settings.suggest_smart_commit = false);
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
        if !self.settings.values().confirm_sync {
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
                    this.settings
                        .update(|settings| settings.confirm_sync = false);
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
                div().flex_1().min_h_0().child(
                    h_resizable("workbench-panels")
                        .child(
                            resizable_panel()
                                .size(px(300.))
                                .child(self.render_scm_panel(repository, window, cx)),
                        )
                        .child(resizable_panel().child(self.render_editor(repository, cx))),
                ),
            )
            .child(self.render_status_bar(repository, cx))
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
                None => this.child(self.render_welcome(cx)),
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

fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}
