//! The branch flows, driven through the window.
use super::*;
use crate::workbench::quick_input::{QuickInput, QuickPick};
use crate::{
    AbortMerge, AbortRebase, CreateBranch, DeleteBranch, DeleteRemoteBranch, MergeBranch,
    RebaseBranch, RenameBranch,
};
use ChangeGroup::Merge;
use core::prelude::v1::test;
use std::fs;

/// `main` and `dev`, where `dev` changed the first line of `a.txt`. A local
/// edit to its last line therefore blocks the checkout, yet merges cleanly.
fn branch_repository() -> TempDir {
    let repository = new_repository();
    let path = repository.path();
    write(path, "a.txt", "1\n2\n3\n");
    write(path, "b.txt", "b\n");
    commit_all(path);
    git(path, &["checkout", "--quiet", "-b", "dev"]);
    write(path, "a.txt", "one\n2\n3\n");
    commit_all(path);
    git(path, &["checkout", "--quiet", "main"]);
    repository
}

fn head(path: &Path) -> String {
    git_output(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_owned()
}

impl Harness {
    /// Waits until a picker shows its items, and returns it.
    async fn wait_for_pick(&self, cx: &mut TestAppContext) -> Entity<QuickPick> {
        let view = self.view.clone();
        cx.wait_for(self.handle, TIMEOUT, move |_, cx| {
            matches!(&view.read(cx).quick_input,
                Some((QuickInput::Pick(pick), _)) if !pick.read(cx).is_loading(cx))
        })
        .await;
        self.quick_pick(cx).expect("a picker")
    }

    /// Waits for a picker, then confirms the item labelled `label`.
    async fn choose(&self, label: &str, cx: &mut TestAppContext) {
        let pick = self.wait_for_pick(cx).await;
        self.update(cx, |window, cx| window.render_frame(cx));
        pick.update(cx, |pick, cx| pick.choose(label, cx));
        cx.run_until_parked();
    }

    async fn wait_for_branch(&self, label: &str, cx: &mut TestAppContext) {
        let label = label.to_owned();
        cx.wait_for(self.handle, TIMEOUT, move |window, _| {
            window
                .try_find("status-branch")
                .is_some_and(|branch| branch.label() == Some(label.as_str()))
        })
        .await;
    }
}

#[gpui_kit::test]
async fn clicking_the_branch_checks_out_another_one(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.click("status-branch", cx);
    let pick = harness.wait_for_pick(cx).await;
    assert_eq!(
        pick.read_with(cx, |pick, _| pick.placeholder().clone()),
        "选择要签出的分支或标记"
    );
    let shown = harness.shown(cx);
    assert_eq!(
        &shown[..4],
        ["创建新分支...", "创建新分支依据...", "签出已分离…", "—"]
    );
    assert!(shown.contains(&"dev [分支]".to_owned()) || shown.contains(&"main [分支]".to_owned()));

    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("dev", cx).await;
    assert_eq!(head(path), "dev");
}

#[gpui_kit::test]
async fn a_remote_branch_is_tracked_or_its_local_branch_checked_out(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let remote = add_remote(path);
    git(path, &["push", "--quiet", "origin", "dev"]);
    let clone = crate::git::fixtures::clone_of(remote.path());
    let path = clone.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    // No local branch tracks origin/dev yet: one is created.
    harness.click("status-branch", cx);
    harness.choose("origin/dev", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("dev", cx).await;
    assert_eq!(harness.label("status-sync", cx).as_deref(), Some("0↓ 0↑"));

    // Now one does: choosing origin/dev from main checks it out again.
    git(path, &["checkout", "--quiet", "main"]);
    harness.press("cmd-r", cx);
    harness.wait_for_branch("main", cx).await;
    harness.click("status-branch", cx);
    harness.choose("origin/dev", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("dev", cx).await;
    let branches = git_output(path, &["branch", "--format=%(refname:short)"]);
    assert_eq!(branches.lines().collect::<Vec<_>>(), ["dev", "main"]);
}

/// Opens the checkout picker over a local edit `dev` would overwrite and
/// answers the question with `answer`.
async fn checkout_over_local_changes(answer: &str, cx: &mut TestAppContext) -> (TempDir, Harness) {
    let repository = branch_repository();
    let path = repository.path();
    write(path, "a.txt", "1\n2\nthree\n");
    write(path, "new.txt", "new\n");
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "new.txt")], cx)
        .await;
    harness.click("status-branch", cx);
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    assert!(cx.has_pending_prompt(), "签出会覆盖本地更改。");
    cx.simulate_prompt_answer(answer);
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    (repository, harness)
}

#[gpui_kit::test]
async fn migrating_changes_carries_them_to_the_other_branch(cx: &mut TestAppContext) {
    let (repository, harness) = checkout_over_local_changes("迁移更改", cx).await;
    let path = repository.path();
    harness.wait_for_branch("dev*", cx).await;
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "one\n2\nthree\n"
    );
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "new.txt")], cx)
        .await;
    assert_eq!(git_output(path, &["stash", "list"]), "");
}

#[gpui_kit::test]
async fn stashing_leaves_the_changes_in_the_stash(cx: &mut TestAppContext) {
    let (repository, harness) = checkout_over_local_changes("储藏并签出", cx).await;
    let path = repository.path();
    harness.wait_for_branch("dev", cx).await;
    harness.wait_for(&[], cx).await;
    assert_eq!(git_output(path, &["stash", "list"]).lines().count(), 1);
}

#[gpui_kit::test]
async fn forcing_the_checkout_discards_the_changes(cx: &mut TestAppContext) {
    let (repository, harness) = checkout_over_local_changes("强制签出", cx).await;
    let path = repository.path();
    harness.wait_for_branch("dev", cx).await;
    harness.wait_for(&[], cx).await;
    assert!(!path.join("new.txt").exists());
    assert_eq!(git_output(path, &["stash", "list"]), "");
}

#[gpui_kit::test]
async fn cancelling_leaves_everything_as_it_was(cx: &mut TestAppContext) {
    let (repository, harness) = checkout_over_local_changes("取消", cx).await;
    let path = repository.path();
    assert_eq!(head(path), "main");
    harness
        .wait_for(&[(Changes, "a.txt"), (Changes, "new.txt")], cx)
        .await;
}

impl Harness {
    fn dispatch(&self, action: impl Action, cx: &mut TestAppContext) {
        self.update(cx, |window, cx| {
            window.dispatch_action(Box::new(action), cx)
        });
        cx.run_until_parked();
    }

    /// Waits until the input box is up, then types `text` and presses Enter.
    async fn answer_input(&self, text: &str, cx: &mut TestAppContext) {
        let view = self.view.clone();
        cx.wait_for(self.handle, TIMEOUT, move |_, cx| {
            matches!(view.read(cx).quick_input, Some((QuickInput::Input(_), _)))
        })
        .await;
        self.key("cmd-a", cx);
        self.type_text(text, cx);
        self.key("enter", cx);
    }

    fn has_element(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.update(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
    }

    async fn wait_for_element(&self, id: &'static str, cx: &mut TestAppContext) {
        cx.wait_for(self.handle, TIMEOUT, move |window, _| {
            window.try_find(id).is_some()
        })
        .await;
    }
}

#[gpui_kit::test]
async fn a_name_typed_in_the_checkout_picker_creates_the_branch(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.click("status-branch", cx);
    harness.wait_for_pick(cx).await;
    harness.type_text("feature/login", cx);
    harness.choose("创建新分支...", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("feature/login", cx).await;
    // A new branch has no upstream: the button offers to publish it.
    harness.wait_for_element("publish-branch", cx).await;
    assert_eq!(
        harness.label("status-sync", cx).as_deref(),
        Some("发布分支")
    );
}

#[gpui_kit::test]
async fn creating_a_branch_asks_for_its_name(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.click("status-branch", cx);
    harness.choose("创建新分支...", cx).await;
    harness.answer_input("fix login", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("fix-login", cx).await;

    // An existing name is refused: Enter leaves the box open.
    harness.dispatch(CreateBranch, cx);
    harness.answer_input("dev", cx).await;
    assert!(harness.has_quick_input(cx));
    harness.key("escape", cx);
    assert_eq!(head(path), "fix-login");
}

#[gpui_kit::test]
async fn creating_a_branch_from_another_start_point(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.click("status-branch", cx);
    harness.choose("创建新分支依据...", cx).await;
    let pick = harness.wait_for_pick(cx).await;
    assert_eq!(
        pick.read_with(cx, |pick, _| pick.placeholder().clone()),
        "选择一个 ref 以从中创建分支"
    );
    assert_eq!(harness.shown(cx)[0], "HEAD");
    harness.choose("dev", cx).await;
    harness.answer_input("hotfix", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("hotfix", cx).await;
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "one\n2\n3\n"
    );
}

#[gpui_kit::test]
async fn checking_out_detached_shows_the_commit(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    git(path, &["tag", "v1"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.click("status-branch", cx);
    harness.choose("签出已分离…", cx).await;
    let pick = harness.wait_for_pick(cx).await;
    assert_eq!(
        pick.read_with(cx, |pick, _| pick.placeholder().clone()),
        "选择要在分离模式下签出的分支"
    );
    let shown = harness.shown(cx);
    assert!(
        !shown
            .iter()
            .any(|label| label.starts_with("创建") || label.starts_with("v1"))
    );
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    let dev = git_output(path, &["rev-parse", "dev"]);
    harness.wait_for_branch(&dev[..8], cx).await;
    assert_eq!(head(path), "HEAD");

    // Detached, there is no branch to sync or publish.
    assert!(!harness.has_element("status-sync", cx));
    harness.click("scm-more", cx);
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        let menu = window.within(MENU);
        assert!(menu.try_find_by_label("推送").is_some());
        assert!(menu.try_find_by_label("发布分支").is_none());
    });
}

#[gpui_kit::test]
async fn renaming_the_current_branch(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.dispatch(RenameBranch, cx);
    harness.answer_input("trunk", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for_branch("trunk", cx).await;

    // Detached, there is no branch to rename.
    git(path, &["checkout", "--quiet", "--detach"]);
    harness.press("cmd-r", cx);
    harness.wait_until_idle(cx).await;
    let commit = git_output(path, &["rev-parse", "HEAD"]);
    harness.wait_for_branch(&commit[..8], cx).await;
    harness.dispatch(RenameBranch, cx);
    assert!(!harness.has_quick_input(cx));
}

#[gpui_kit::test]
async fn deleting_an_unmerged_branch_asks_again(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;
    let branches = || git_output(path, &["branch", "--format=%(refname:short)"]);

    harness.dispatch(DeleteBranch, cx);
    harness.wait_for_pick(cx).await;
    // The current branch cannot be deleted, so it is not offered.
    assert_eq!(harness.shown(cx), ["dev [分支]"]);
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    cx.run_until_parked();
    assert_eq!(branches(), "dev\nmain\n");

    harness.dispatch(DeleteBranch, cx);
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    cx.simulate_prompt_answer("删除分支");
    cx.run_until_parked();
    harness.wait_until_idle(cx).await;
    assert_eq!(branches(), "main\n");
}

#[gpui_kit::test]
async fn deleting_a_remote_branch(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let remote = add_remote(path);
    git(path, &["push", "--quiet", "origin", "dev"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.dispatch(DeleteRemoteBranch, cx);
    harness.wait_for_pick(cx).await;
    // The current branch's upstream is not offered.
    assert_eq!(harness.shown(cx), ["origin/dev [远程分支]"]);
    harness.choose("origin/dev", cx).await;
    harness.wait_until_idle(cx).await;
    let remote_heads = git_output(remote.path(), &["branch", "--format=%(refname:short)"]);
    assert_eq!(remote_heads, "main\n");
}

/// `main` and `dev` both changed the first line of `a.txt`.
fn conflicting_repository() -> TempDir {
    let repository = branch_repository();
    let path = repository.path();
    write(path, "a.txt", "uno\n2\n3\n");
    commit_all(path);
    repository
}

#[gpui_kit::test]
async fn merging_without_and_with_conflicts(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    git(path, &["checkout", "--quiet", "-b", "side", "main"]);
    write(path, "c.txt", "c\n");
    commit_all(path);
    git(path, &["checkout", "--quiet", "main"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.dispatch(MergeBranch, cx);
    let pick = harness.wait_for_pick(cx).await;
    assert_eq!(
        pick.read_with(cx, |pick, _| pick.placeholder().clone()),
        "选择要合并的分支或标记"
    );
    harness.choose("side", cx).await;
    harness.wait_until_idle(cx).await;
    assert!(path.join("c.txt").exists());

    // Now `dev` conflicts with `main`.
    write(path, "a.txt", "uno\n2\n3\n");
    commit_all(path);
    harness.press("cmd-r", cx);
    harness.wait_until_idle(cx).await;
    let notifications = harness.notification_count(cx);
    harness.dispatch(MergeBranch, cx);
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Merge, "a.txt")], cx).await;
    assert_eq!(harness.notification_count(cx), notifications + 1);
    harness.wait_for_element("continue", cx).await;
}

#[gpui_kit::test]
async fn continuing_a_merge_after_resolving_it(cx: &mut TestAppContext) {
    let repository = conflicting_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;
    harness.dispatch(MergeBranch, cx);
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Merge, "a.txt")], cx).await;
    assert!(harness.message(cx).starts_with("Merge branch 'dev'"));

    // Unresolved, 继续 does nothing.
    harness.click("continue", cx);
    harness.wait_until_idle(cx).await;
    assert!(harness.has_element("continue", cx));

    write(path, "a.txt", "resolved\n2\n3\n");
    harness.click_row_action(file_row_id(Merge, "a.txt"), "stage-merge-a.txt", cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Staged, "a.txt")], cx).await;
    harness.click("continue", cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    let parents = git_output(path, &["log", "-1", "--format=%P"]);
    assert_eq!(parents.split_whitespace().count(), 2);
    assert!(!harness.has_element("continue", cx));
}

#[gpui_kit::test]
async fn rebasing_lists_the_upstream_first_and_continues_or_aborts(cx: &mut TestAppContext) {
    let repository = conflicting_repository();
    let path = repository.path();
    let _remote = add_remote(path);
    git(path, &["checkout", "--quiet", "dev"]);
    let before = git_output(path, &["rev-parse", "HEAD"]);
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    harness.dispatch(RebaseBranch, cx);
    harness.wait_for_pick(cx).await;
    assert_eq!(harness.shown(cx)[0], "main [分支]");
    harness.key("escape", cx);

    // On `main`, whose upstream is origin/main, the upstream comes first.
    git(path, &["checkout", "--quiet", "main"]);
    harness.press("cmd-r", cx);
    harness.wait_for_branch("main", cx).await;
    harness.dispatch(RebaseBranch, cx);
    harness.wait_for_pick(cx).await;
    assert_eq!(&harness.shown(cx)[..2], ["origin/main", "—"]);
    harness.key("escape", cx);

    // Rebasing `dev` onto `main` stops; aborting returns to where it was.
    git(path, &["checkout", "--quiet", "dev"]);
    harness.press("cmd-r", cx);
    harness.wait_for_branch("dev", cx).await;
    harness.dispatch(RebaseBranch, cx);
    harness.choose("main", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Merge, "a.txt")], cx).await;
    harness.dispatch(AbortRebase, cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    assert_eq!(git_output(path, &["rev-parse", "HEAD"]), before);

    // Again, resolving and continuing finishes it.
    harness.dispatch(RebaseBranch, cx);
    harness.choose("main", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Merge, "a.txt")], cx).await;
    write(path, "a.txt", "resolved\n2\n3\n");
    git(path, &["add", "a.txt"]);
    harness.press("cmd-r", cx);
    harness.wait_for(&[(Staged, "a.txt")], cx).await;
    harness.click("continue", cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    harness.wait_for_branch("dev", cx).await;
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "resolved\n2\n3\n"
    );
}

#[gpui_kit::test]
async fn aborting_a_merge(cx: &mut TestAppContext) {
    let repository = conflicting_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;
    harness.dispatch(MergeBranch, cx);
    harness.choose("dev", cx).await;
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[(Merge, "a.txt")], cx).await;
    harness.dispatch(AbortMerge, cx);
    harness.wait_until_idle(cx).await;
    harness.wait_for(&[], cx).await;
    assert!(!harness.has_element("continue", cx));
}

#[gpui_kit::test]
async fn publishing_to_the_only_remote_or_a_chosen_one(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    // No remote: a warning, nothing pushed.
    harness.wait_for_element("publish-branch", cx).await;
    let notifications = harness.notification_count(cx);
    harness.click("publish-branch", cx);
    cx.run_until_parked();
    cx.wait_for(harness.handle, TIMEOUT, move |window, cx| {
        window.notifications(cx).len() == notifications + 1
    })
    .await;

    // One remote: pushed there.
    let _origin = add_remote(path);
    git(path, &["checkout", "--quiet", "-b", "feature"]);
    harness.press("cmd-r", cx);
    harness.wait_for_branch("feature", cx).await;
    harness.click("publish-branch", cx);
    harness.wait_until_idle(cx).await;
    cx.wait_for(harness.handle, TIMEOUT, |window, _| {
        window
            .try_find("status-sync")
            .is_some_and(|sync| sync.label() == Some("0↓ 0↑"))
    })
    .await;

    // Two remotes: the user picks one.
    let other = tempfile::tempdir().expect("temporary directory");
    git(other.path(), &["init", "--quiet", "--bare"]);
    git(
        path,
        &["remote", "add", "upstream", other.path().to_str().unwrap()],
    );
    git(path, &["checkout", "--quiet", "-b", "second"]);
    harness.press("cmd-r", cx);
    harness.wait_for_branch("second", cx).await;
    harness.click("publish-branch", cx);
    let pick = harness.wait_for_pick(cx).await;
    assert_eq!(
        pick.read_with(cx, |pick, _| pick.placeholder().clone()),
        "选取要将分支“second”发布到的远程:"
    );
    assert_eq!(harness.shown(cx), ["origin", "upstream"]);
    harness.choose("upstream", cx).await;
    harness.wait_until_idle(cx).await;
    let upstream = git_output(path, &["rev-parse", "--abbrev-ref", "second@{upstream}"]);
    assert_eq!(upstream.trim(), "upstream/second");
}

/// The popover the 「…」 button opens; submenus open inside it.
const MENU: &str = "popover:dropdown-menu:Name(\"scm-more\")";

#[gpui_kit::test]
async fn the_more_menu_follows_vs_code(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let harness = open_window(Some(repository.path().to_path_buf()), cx);
    harness.wait_for(&[], cx).await;
    harness.click("scm-more", cx);
    let tops = |harness: &Harness, labels: &[&str], cx: &mut TestAppContext| {
        harness.update(cx, |window, cx| {
            window.render_frame(cx);
            let menu = window.within(MENU);
            labels
                .iter()
                .map(|label| menu.find_by_label(label).bounds().origin.y)
                .collect::<Vec<_>>()
        })
    };
    let ordered = |ys: Vec<Pixels>| ys.windows(2).all(|pair| pair[0] < pair[1]);
    assert!(ordered(tops(
        &harness,
        &[
            "拉取",
            "发布分支",
            "签出到...",
            "抓取",
            "提交",
            "更改",
            "拉取，推送",
            "分支"
        ],
        cx
    )));

    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        let target = window.within(MENU).find_by_label("分支");
        let id = target.path().last().cloned().expect("target id");
        window.hover(id, cx);
    });
    cx.run_until_parked();
    assert!(ordered(tops(
        &harness,
        &[
            "合并...",
            "变基分支…",
            "创建分支...",
            "从现有来源创建新的分支...",
            "重命名分支...",
            "删除分支...",
            "删除远程分支...",
            "发布分支...",
        ],
        cx
    )));
}

#[gpui_kit::test]
async fn branch_actions_wait_while_a_change_runs(cx: &mut TestAppContext) {
    let repository = branch_repository();
    let path = repository.path();
    let _remote = add_remote(path);
    // Every fetch from this remote takes two seconds.
    git(
        path,
        &[
            "config",
            "remote.origin.uploadpack",
            "sleep 2; git-upload-pack",
        ],
    );
    let harness = open_window(Some(path.to_path_buf()), cx);
    harness.wait_for(&[], cx).await;

    // Parking would wait for the fetch itself, so nothing here parks until
    // the checks are done.
    // An action is dispatched when the update ends, so the fetch is under
    // way before the next one clicks.
    harness.update(cx, |window, cx| {
        window.dispatch_action(Box::new(crate::Fetch), cx)
    });
    harness.update(cx, |window, cx| {
        window.render_frame(cx);
        window.click("status-branch", cx);
        window.dispatch_action(Box::new(crate::Checkout), cx);
    });
    assert!(harness.view.read_with(cx, |view, _| {
        view.repository
            .as_ref()
            .is_some_and(|repository| repository.is_busy())
    }));

    // Once the fetch is done and every queued task has run, still no picker:
    // the click and the action were refused, not merely delayed.
    harness.wait_until_idle(cx).await;
    cx.run_until_parked();
    assert!(
        !harness.has_quick_input(cx),
        "nothing opened while fetching"
    );

    harness.dispatch(crate::Checkout, cx);
    harness.wait_for_pick(cx).await;
}
