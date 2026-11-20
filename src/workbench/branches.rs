//! The branch flows of VS Code's Source Control: checking out, creating,
//! renaming, deleting, merging, rebasing and publishing, and continuing or
//! aborting a merge or rebase that stopped. Each flow is one async function
//! that asks its questions in order and then runs git, as VS Code's commands do.
use std::path::{Path, PathBuf};

use gpui_kit::component::{Icon, WindowExt as _, notification::Notification};
use gpui_kit::*;

use super::{
    Operation, Workbench,
    quick_input::{self, InputOptions, PickGroup, PickItem, PickList, Validation},
    relative_time::{now, relative_time},
    report_failure,
};
use crate::codicons::Codicon;
use crate::git::{self, GitError, Head, InProgress, RefKind, Reference, Remote};

/// What a flow needs to know about the repository when it starts.
#[derive(Clone)]
struct BranchContext {
    root: PathBuf,
    /// The checked-out branch; `None` when HEAD is detached.
    branch: Option<String>,
    /// Its upstream, such as `origin/main`.
    upstream: Option<String>,
}

/// What VS Code's checkout picker offers.
enum CheckoutChoice {
    CreateBranch,
    CreateBranchFrom,
    Detached,
    Target(CheckoutTarget),
}

/// The git checkout a picked reference stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CheckoutTarget {
    /// A local branch, or a tag checked out detached.
    Name(String),
    /// A remote branch no local branch tracks yet: create one that does.
    Track(String),
    Detached(String),
}

/// A git change to run in the background.
type GitWork = Box<dyn FnOnce(&Path) -> Result<(), GitError> + Send>;

/// The answers to "签出会覆盖本地更改。", in button order.
const STASH_AND_CHECKOUT: usize = 0;
const MIGRATE_CHANGES: usize = 1;
const FORCE_CHECKOUT: usize = 2;

const CONFLICT_MESSAGE: &str = "存在合并冲突。请在提交之前解决这些冲突。";

impl Workbench {
    /// What a branch flow starts from, or `None` while a change is running.
    fn branch_context(&self) -> Option<BranchContext> {
        let repository = self.repository.as_ref()?;
        if repository.is_busy() {
            return None;
        }
        let branch = repository.branch()?;
        Some(BranchContext {
            root: repository.root.clone(),
            branch: match &branch.head {
                Head::Branch(name) => Some(name.clone()),
                Head::Detached(_) => None,
            },
            upstream: branch
                .upstream
                .as_ref()
                .map(|upstream| upstream.name.clone()),
        })
    }

    /// Runs `flow` with the repository as it is now, unless a change is
    /// already running.
    fn start_flow<F>(&mut self, window: &mut Window, cx: &mut Context<Self>, flow: F)
    where
        F: AsyncFnOnce(WeakEntity<Workbench>, &mut AsyncWindowContext, BranchContext) -> Option<()>
            + 'static,
    {
        let Some(context) = self.branch_context() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            flow(this, cx, context).await;
        })
        .detach();
    }

    /// 签出到...: the picker of branches, remote branches and tags.
    pub(super) fn checkout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_flow(window, cx, async |this, cx, context| {
            checkout_flow(&this, cx, context, false).await
        });
    }

    /// 创建分支... or 从现有来源创建新的分支...
    pub(super) fn create_branch(
        &mut self,
        from: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_flow(window, cx, async move |this, cx, context| {
            create_branch_flow(&this, cx, &context, String::new(), from).await
        });
    }

    pub(super) fn rename_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_flow(window, cx, async |this, cx, context| {
            rename_branch_flow(&this, cx, context).await
        });
    }

    pub(super) fn delete_branch(
        &mut self,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_flow(window, cx, async move |this, cx, context| {
            if remote {
                delete_remote_branch_flow(&this, cx, context).await
            } else {
                delete_branch_flow(&this, cx, context).await
            }
        });
    }

    pub(super) fn merge_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_flow(window, cx, async |this, cx, context| {
            merge_flow(&this, cx, context).await
        });
    }

    pub(super) fn rebase_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_flow(window, cx, async |this, cx, context| {
            rebase_flow(&this, cx, context).await
        });
    }

    /// Pushes the current branch and makes the remote branch its upstream.
    pub(super) fn publish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_flow(window, cx, async |this, cx, context| {
            publish_flow(&this, cx, context).await
        });
    }

    /// 继续: concludes the merge with a commit, or continues the rebase.
    pub(super) fn continue_in_progress(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let in_progress = self
            .repository
            .as_ref()
            .and_then(|repository| repository.snapshot.as_ref())
            .and_then(|snapshot| snapshot.in_progress);
        match in_progress {
            Some(InProgress::Merge) => self.commit(false, super::AfterCommit::Nothing, window, cx),
            Some(InProgress::Rebase) => self.start_flow(window, cx, async |this, cx, _| {
                let result =
                    run(&this, cx, Operation::ContinueRebase, git::rebase_continue).await?;
                finish(&this, cx, Operation::ContinueRebase, result);
                Some(())
            }),
            None => {}
        }
    }

    /// 中止合并 / 中止变基.
    pub(super) fn abort_in_progress(
        &mut self,
        in_progress: InProgress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match in_progress {
            InProgress::Merge => self.run_operation(
                Operation::AbortMerge,
                git::merge_abort,
                |_, _, _| {},
                window,
                cx,
            ),
            InProgress::Rebase => self.run_operation(
                Operation::AbortRebase,
                git::rebase_abort,
                |_, _, _| {},
                window,
                cx,
            ),
        }
    }
}

async fn checkout_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
    detached: bool,
) -> Option<()> {
    let root = context.root.clone();
    let list = cx.background_spawn(async move {
        git::list_references(&root).map(|references| checkout_list(&references, now(), detached))
    });
    let placeholder = if detached {
        "选择要在分离模式下签出的分支"
    } else {
        "选择要签出的分支或标记"
    };
    let picked = quick_input::pick(this, cx, placeholder, list).await?;
    match picked.value {
        CheckoutChoice::CreateBranch => {
            create_branch_flow(this, cx, &context, picked.query, false).await
        }
        CheckoutChoice::CreateBranchFrom => {
            create_branch_flow(this, cx, &context, picked.query, true).await
        }
        CheckoutChoice::Detached => Box::pin(checkout_flow(this, cx, context, true)).await,
        CheckoutChoice::Target(target) => checkout_target(this, cx, target).await,
    }
}

/// Checks out `target`; when local changes stand in the way, asks VS Code's
/// question and stashes or discards them first.
async fn checkout_target(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    target: CheckoutTarget,
) -> Option<()> {
    let first = target.clone();
    let result = run(this, cx, Operation::Checkout, move |root| first.run(root)).await?;
    let Err(GitError::DirtyWorkTree) = result else {
        finish(this, cx, Operation::Checkout, result);
        return Some(());
    };
    let answer = ask(
        this,
        cx,
        PromptLevel::Warning,
        "签出会覆盖本地更改。",
        None,
        &["储藏并签出", "迁移更改", "强制签出", "取消"],
    )
    .await?;
    let work: GitWork = match answer {
        STASH_AND_CHECKOUT => Box::new(move |root| {
            git::stash_all(root)?;
            target.run(root)
        }),
        MIGRATE_CHANGES => Box::new(move |root| {
            git::stash_all(root)?;
            target.run(root)?;
            git::stash_pop(root)
        }),
        FORCE_CHECKOUT => Box::new(move |root| {
            git::discard_everything(root)?;
            target.run(root)
        }),
        _ => return None,
    };
    let result = run(this, cx, Operation::Checkout, work).await?;
    finish(this, cx, Operation::Checkout, result);
    Some(())
}

impl CheckoutTarget {
    fn run(&self, root: &Path) -> Result<(), GitError> {
        match self {
            CheckoutTarget::Name(name) => git::checkout(root, name),
            CheckoutTarget::Track(remote_branch) => git::checkout_tracking(root, remote_branch),
            CheckoutTarget::Detached(commit) => git::checkout_detached(root, commit),
        }
    }
}

/// VS Code's checkout picker. In detached mode there are no actions and no
/// tags, and every branch checks out its commit.
fn checkout_list(references: &[Reference], now: i64, detached: bool) -> PickList<CheckoutChoice> {
    let actions = if detached {
        Vec::new()
    } else {
        vec![
            (
                PickItem::new("创建新分支...").icon(Codicon::Add),
                CheckoutChoice::CreateBranch,
            ),
            (
                PickItem::new("创建新分支依据...").icon(Codicon::Add),
                CheckoutChoice::CreateBranchFrom,
            ),
            (
                PickItem::new("签出已分离…").icon(Icon::new(Codicon::DebugDisconnect)),
                CheckoutChoice::Detached,
            ),
        ]
    };
    let target = |reference: &Reference| {
        if detached {
            return CheckoutTarget::Detached(reference.commit.clone());
        }
        match &reference.kind {
            RefKind::Local | RefKind::Tag => CheckoutTarget::Name(reference.name.clone()),
            RefKind::Remote { .. } => references
                .iter()
                .find(|local| {
                    local.kind == RefKind::Local
                        && local.upstream.as_deref() == Some(reference.name.as_str())
                })
                .map_or_else(
                    || CheckoutTarget::Track(reference.name.clone()),
                    |local| CheckoutTarget::Name(local.name.clone()),
                ),
        }
    };
    let mut kinds = vec![GroupKind::Local, GroupKind::Remote];
    if !detached {
        kinds.push(GroupKind::Tag);
    }
    PickList {
        actions,
        groups: reference_groups(references, now, &kinds, |reference| {
            CheckoutChoice::Target(target(reference))
        }),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    Local,
    Remote,
    Tag,
}

impl GroupKind {
    fn of(kind: &RefKind) -> Self {
        match kind {
            RefKind::Local => GroupKind::Local,
            RefKind::Remote { .. } => GroupKind::Remote,
            RefKind::Tag => GroupKind::Tag,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GroupKind::Local => "分支",
            GroupKind::Remote => "远程分支",
            GroupKind::Tag => "标记",
        }
    }
}

/// One group per kind in `kinds`, in that order, each in git's newest-first
/// order; empty groups are left out.
fn reference_groups<T>(
    references: &[Reference],
    now: i64,
    kinds: &[GroupKind],
    value: impl Fn(&Reference) -> T,
) -> Vec<PickGroup<T>> {
    kinds
        .iter()
        .map(|&kind| PickGroup {
            label: Some(kind.label().into()),
            items: references
                .iter()
                .filter(|reference| GroupKind::of(&reference.kind) == kind)
                .map(|reference| (reference_item(reference, now), value(reference)))
                .collect(),
        })
        .filter(|group| !group.items.is_empty())
        .collect()
}

/// A reference as VS Code lists it: its counts against the upstream and how
/// long ago it was committed, then author, hash and subject below.
fn reference_item(reference: &Reference, now: i64) -> PickItem {
    let icon = match reference.kind {
        RefKind::Local => Icon::new(Codicon::GitBranch),
        RefKind::Remote { .. } => Icon::new(Codicon::Cloud),
        RefKind::Tag => Icon::new(Codicon::Tag),
    };
    let short = reference.short_commit();
    let mut parts = Vec::new();
    if let Some((ahead, behind)) = reference.track {
        parts.push(format!("{behind}↓ {ahead}↑"));
    }
    if let Some(date) = reference.date {
        parts.push(relative_time(date, now));
    }
    let description = if parts.is_empty() {
        match reference.kind {
            RefKind::Local => short.to_owned(),
            RefKind::Remote { .. } => format!("{short} 处的远程分支"),
            RefKind::Tag => format!("{short} 处的标记"),
        }
    } else {
        parts.join(" • ")
    };
    let item = PickItem::new(reference.name.clone())
        .icon(icon)
        .description(description);
    if reference.author.is_empty() || reference.subject.is_empty() {
        item
    } else {
        item.detail(format!(
            "{} • {short} • {}",
            reference.author, reference.subject
        ))
    }
}

/// Creates and checks out a branch. A name typed in the checkout picker is
/// used as is; otherwise the input box asks for one. With `from`, the start
/// point is picked first.
async fn create_branch_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: &BranchContext,
    typed: String,
    from: bool,
) -> Option<()> {
    let start = if from {
        let root = context.root.clone();
        let list = cx.background_spawn(async move {
            let references = git::list_references(&root)?;
            let head = git::head_commit(&root)?;
            Ok(start_point_list(&references, head.as_deref(), now()))
        });
        quick_input::pick(this, cx, "选择一个 ref 以从中创建分支", list)
            .await?
            .value
    } else {
        "HEAD".to_owned()
    };
    let name = if typed.trim().is_empty() {
        let typed = ask_branch_name(this, cx, &context.root, String::new()).await?;
        git::sanitize_branch_name(&typed)
    } else {
        git::sanitize_branch_name(&typed)
    };
    if name.is_empty() {
        return None;
    }
    let result = run(this, cx, Operation::CreateBranch, move |root| {
        git::create_branch(root, &name, &start)
    })
    .await?;
    finish(this, cx, Operation::CreateBranch, result);
    Some(())
}

/// `HEAD` first, then every branch, remote branch and tag to start from.
fn start_point_list(references: &[Reference], head: Option<&str>, now: i64) -> PickList<String> {
    let mut list = PickList {
        actions: Vec::new(),
        groups: reference_groups(
            references,
            now,
            &[GroupKind::Local, GroupKind::Remote, GroupKind::Tag],
            |reference| reference.name.clone(),
        ),
    };
    let head_item = PickItem::new("HEAD").description(
        head.map(|commit| commit[..commit.len().min(7)].to_owned())
            .unwrap_or_default(),
    );
    list.actions.push((head_item, "HEAD".to_owned()));
    list
}

/// The input box for a branch name, checked against the existing branches.
async fn ask_branch_name(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    root: &Path,
    value: String,
) -> Option<String> {
    let root = root.to_owned();
    let existing: Vec<String> = cx
        .background_spawn(async move { git::list_references(&root) })
        .await
        .map(|references| {
            references
                .into_iter()
                .filter(|reference| reference.kind == RefKind::Local)
                .map(|reference| reference.name)
                .collect()
        })
        .unwrap_or_default();
    quick_input::input(
        this,
        cx,
        InputOptions {
            placeholder: "分支名称".into(),
            prompt: "请提供新的分支名称".into(),
            value,
            validate: Box::new(move |value| validate_branch_name(value, &existing)),
        },
    )
    .await
}

fn validate_branch_name(value: &str, existing: &[String]) -> Validation {
    let name = git::sanitize_branch_name(value);
    if existing.contains(&name) {
        Validation::Error(format!("分支“{name}”已存在").into())
    } else if name != value {
        Validation::Info(format!("新分支将为“{name}”").into())
    } else {
        Validation::Valid
    }
}

async fn rename_branch_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
) -> Option<()> {
    let current = context.branch?;
    let typed = ask_branch_name(this, cx, &context.root, current).await?;
    let name = git::sanitize_branch_name(&typed);
    if name.is_empty() {
        return None;
    }
    let result = run(this, cx, Operation::RenameBranch, move |root| {
        git::rename_branch(root, &name)
    })
    .await?;
    finish(this, cx, Operation::RenameBranch, result);
    Some(())
}

/// The references a branch operation may target: those of `kinds`, without
/// the current branch and its upstream.
fn other_references(context: &BranchContext, references: Vec<Reference>) -> Vec<Reference> {
    references
        .into_iter()
        .filter(|reference| {
            Some(&reference.name)
                != match reference.kind {
                    RefKind::Local => context.branch.as_ref(),
                    RefKind::Remote { .. } => context.upstream.as_ref(),
                    RefKind::Tag => None,
                }
        })
        .collect()
}

async fn delete_branch_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
) -> Option<()> {
    let root = context.root.clone();
    let list = cx.background_spawn(async move {
        let references = other_references(&context, git::list_references(&root)?);
        Ok(PickList {
            actions: Vec::new(),
            groups: reference_groups(&references, now(), &[GroupKind::Local], |reference| {
                reference.name.clone()
            }),
        })
    });
    let name = quick_input::pick(this, cx, "选择要删除的分支", list)
        .await?
        .value;
    let first = name.clone();
    let result = run(this, cx, Operation::DeleteBranch, move |root| {
        git::delete_branch(root, &first, false)
    })
    .await?;
    let Err(GitError::BranchNotFullyMerged) = result else {
        finish(this, cx, Operation::DeleteBranch, result);
        return Some(());
    };
    let answer = ask(
        this,
        cx,
        PromptLevel::Warning,
        &format!("“{name}”分支未被完全合并。是否仍要删除?"),
        None,
        &["删除分支", "取消"],
    )
    .await?;
    if answer != 0 {
        return None;
    }
    let result = run(this, cx, Operation::DeleteBranch, move |root| {
        git::delete_branch(root, &name, true)
    })
    .await?;
    finish(this, cx, Operation::DeleteBranch, result);
    Some(())
}

async fn delete_remote_branch_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
) -> Option<()> {
    let root = context.root.clone();
    let list = cx.background_spawn(async move {
        let references = other_references(&context, git::list_references(&root)?);
        Ok(PickList {
            actions: Vec::new(),
            groups: reference_groups(&references, now(), &[GroupKind::Remote], |reference| {
                reference.clone()
            }),
        })
    });
    let reference = quick_input::pick(this, cx, "选择要删除的远程分支", list)
        .await?
        .value;
    let RefKind::Remote { remote } = reference.kind else {
        return None;
    };
    let branch = reference.name[remote.len() + 1..].to_owned();
    let result = run(this, cx, Operation::DeleteRemoteBranch, move |root| {
        git::delete_remote_branch(root, &remote, &branch)
    })
    .await?;
    finish(this, cx, Operation::DeleteRemoteBranch, result);
    Some(())
}

async fn merge_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
) -> Option<()> {
    let root = context.root.clone();
    let list = cx.background_spawn(async move {
        let references = other_references(&context, git::list_references(&root)?);
        Ok(PickList {
            actions: Vec::new(),
            groups: reference_groups(
                &references,
                now(),
                &[GroupKind::Local, GroupKind::Remote, GroupKind::Tag],
                |reference| reference.name.clone(),
            ),
        })
    });
    let name = quick_input::pick(this, cx, "选择要合并的分支或标记", list)
        .await?
        .value;
    let result = run(this, cx, Operation::Merge, move |root| {
        git::merge(root, &name)
    })
    .await?;
    finish(this, cx, Operation::Merge, result);
    Some(())
}

async fn rebase_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
) -> Option<()> {
    let root = context.root.clone();
    let list = cx.background_spawn(async move {
        let references = git::list_references(&root)?;
        Ok(rebase_list(&context, references, now()))
    });
    let name = quick_input::pick(this, cx, "选择要变基到的分支", list)
        .await?
        .value;
    let result = run(this, cx, Operation::Rebase, move |root| {
        git::rebase(root, &name)
    })
    .await?;
    finish(this, cx, Operation::Rebase, result);
    Some(())
}

/// Branches and remote branches to rebase onto, the upstream first.
fn rebase_list(context: &BranchContext, references: Vec<Reference>, now: i64) -> PickList<String> {
    let upstream = context.upstream.as_ref().and_then(|upstream| {
        references
            .iter()
            .find(|reference| {
                matches!(reference.kind, RefKind::Remote { .. }) && &reference.name == upstream
            })
            .map(|reference| {
                (
                    reference_item(reference, now).description("(upstream)"),
                    reference.name.clone(),
                )
            })
    });
    let references = other_references(context, references);
    let mut groups: Vec<PickGroup<String>> = upstream
        .map(|item| PickGroup {
            label: None,
            items: vec![item],
        })
        .into_iter()
        .collect();
    groups.extend(reference_groups(
        &references,
        now,
        &[GroupKind::Local, GroupKind::Remote],
        |reference| reference.name.clone(),
    ));
    PickList {
        actions: Vec::new(),
        groups,
    }
}

/// Publishes to the only remote, or asks which one when there are several.
async fn publish_flow(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    context: BranchContext,
) -> Option<()> {
    let branch = context.branch?;
    let root = context.root.clone();
    let remotes = cx
        .background_spawn(async move { git::remotes(&root) })
        .await;
    let remote = match remotes {
        Err(error) => {
            finish(this, cx, Operation::Publish, Err(error));
            return None;
        }
        Ok(remotes) => match <[Remote; 1]>::try_from(remotes) {
            Ok([only]) => only.name,
            Err(remotes) if remotes.is_empty() => {
                notify(
                    this,
                    cx,
                    Notification::warning(GitError::NoRemote.to_string()),
                );
                return None;
            }
            Err(remotes) => {
                let list = PickList {
                    actions: Vec::new(),
                    groups: vec![PickGroup {
                        label: None,
                        items: remotes
                            .into_iter()
                            .map(|remote| {
                                (
                                    PickItem::new(remote.name.clone()).description(remote.push_url),
                                    remote.name,
                                )
                            })
                            .collect(),
                    }],
                };
                let placeholder = format!("选取要将分支“{branch}”发布到的远程:");
                quick_input::pick(this, cx, placeholder, async { Ok(list) })
                    .await?
                    .value
            }
        },
    };
    let result = run(this, cx, Operation::Publish, move |root| {
        git::publish(root, &remote, &branch)
    })
    .await?;
    finish(this, cx, Operation::Publish, result);
    Some(())
}

/// Starts `work` as the change in progress and waits for its result; `None`
/// when it could not start or the window went away.
async fn run(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    operation: Operation,
    work: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static,
) -> Option<Result<(), GitError>> {
    let result = this
        .update_in(cx, |this, window, cx| {
            this.start_operation(operation, work, window, cx)
        })
        .ok()??;
    result.await.ok()
}

/// Reports a failure: a stop for conflicts as VS Code's warning, anything
/// else as the operation's failure.
fn finish(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    operation: Operation,
    result: Result<(), GitError>,
) {
    let Err(error) = result else {
        return;
    };
    let reported = this.update_in(cx, |_, window, cx| match error {
        GitError::Conflict => window.push_notification(Notification::warning(CONFLICT_MESSAGE), cx),
        error => report_failure(operation, &error, window, cx),
    });
    if let Err(error) = reported {
        log::warn!("view released before reporting {operation:?}: {error:#}");
    }
}

fn notify(this: &WeakEntity<Workbench>, cx: &mut AsyncWindowContext, note: Notification) {
    if let Err(error) = this.update_in(cx, |_, window, cx| window.push_notification(note, cx)) {
        log::warn!("view released before notifying: {error:#}");
    }
}

/// Asks with a native dialog and returns the chosen button's index.
async fn ask(
    this: &WeakEntity<Workbench>,
    cx: &mut AsyncWindowContext,
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    answers: &[&str],
) -> Option<usize> {
    let answer = this
        .update_in(cx, |_, window, cx| {
            window.prompt(level, message, detail, answers, cx)
        })
        .ok()?;
    answer.await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn reference(kind: RefKind, name: &str) -> Reference {
        Reference {
            kind,
            name: name.to_owned(),
            commit: "05c648e0123456789".to_owned(),
            author: "李飞".to_owned(),
            date: Some(1_000_000 - 2 * 86_400),
            subject: "Merge branch 'x' into 'main'".to_owned(),
            upstream: None,
            track: None,
        }
    }

    #[test]
    fn a_branch_shows_counts_age_and_its_commit() {
        let mut main = reference(RefKind::Local, "main");
        main.track = Some((0, 2));
        let item = reference_item(&main, 1_000_000);
        assert_eq!(item.description.as_deref(), Some("2↓ 0↑ • 2 天前"));
        assert_eq!(
            item.detail.as_deref(),
            Some("李飞 • 05c648e • Merge branch 'x' into 'main'")
        );

        let mut tag = reference(RefKind::Tag, "v1");
        tag.date = None;
        assert_eq!(
            reference_item(&tag, 1_000_000).description.as_deref(),
            Some("05c648e 处的标记")
        );
    }

    #[test]
    fn a_remote_branch_checks_out_its_local_tracking_branch() {
        let remote = || RefKind::Remote {
            remote: "origin".into(),
        };
        let mut dev = reference(RefKind::Local, "dev");
        dev.upstream = Some("origin/dev".into());
        let references = vec![
            dev,
            reference(remote(), "origin/dev"),
            reference(remote(), "origin/feature"),
            reference(RefKind::Tag, "v1"),
        ];
        let list = checkout_list(&references, 1_000_000, false);
        let targets: Vec<_> = list
            .groups
            .iter()
            .flat_map(|group| &group.items)
            .map(|(_, choice)| match choice {
                CheckoutChoice::Target(target) => target.clone(),
                _ => panic!("only targets in groups"),
            })
            .collect();
        assert_eq!(
            targets,
            [
                CheckoutTarget::Name("dev".into()),
                CheckoutTarget::Name("dev".into()),
                CheckoutTarget::Track("origin/feature".into()),
                CheckoutTarget::Name("v1".into()),
            ]
        );
        assert_eq!(list.actions.len(), 3);

        let detached = checkout_list(&references, 1_000_000, true);
        assert!(detached.actions.is_empty());
        assert_eq!(detached.groups.len(), 2, "no tags in detached mode");
    }

    #[test]
    fn branch_names_are_checked_against_existing_ones() {
        let existing = vec!["dev".to_owned()];
        assert_eq!(
            validate_branch_name("dev", &existing),
            Validation::Error("分支“dev”已存在".into())
        );
        assert_eq!(
            validate_branch_name("fix login", &existing),
            Validation::Info("新分支将为“fix-login”".into())
        );
        assert_eq!(
            validate_branch_name("feature", &existing),
            Validation::Valid
        );
    }
}
