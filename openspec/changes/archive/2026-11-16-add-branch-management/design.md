## Context

做这件事的原因见 proposal.md 的「Why」，具体行为见 `specs/` 下的四个文件。下面只列和方案有关的现状。

- **写操作**：`Workbench::run_operation(operation, work, then)` 在后台执行一个 `FnOnce(&Path) -> Result<(), GitError>`，同一时间只执行一个（`Repository.operation`）。失败时统一由 `report_failure` 弹出通知，成功时调用 `then`，无论成败结束后都刷新。所有远程命令走 `remote_command`（用 `setsid` 脱离终端，不会卡在等待输入上）。
- **读取**：`git::load_snapshot` 用 `status --porcelain=v2 --branch` 加三份 patch 组成 `Snapshot`；`key` 相同说明仓库没变，此时不更新界面。当前读取里没有「合并 / 变基进行中」这个状态。
- **错误**：`GitError::CommandFailed { command, stderr }` 只保留 stderr。`NoRemote` / `AmbiguousRemote` 是为发布分支加的。
- **用户确认**：一律用 `window.prompt`（macOS 原生确认框），测试里用 `simulate_prompt_answer` 模拟回答。
- **可用的组件**：vendor 的 gpui-component 有 `command` 命令面板（`Command` + `CommandState`），已经支持：输入框、分组、分隔线、不可选条目、条目自定义内容（`CommandItem::child`）、`filterable(false)` 时由调用方自己过滤（配合 `on_query`）、上下键循环移动并跳过不可选条目、回车确认、Esc 取消、`set_loading` 显示加载、按选中项滚动。`PopupMenu::submenu` 支持子菜单。`input::InputState` 用来做输入框模式。
- **VS Code 参照**：取自本机安装的 VS Code 1.117 `extensions/git/dist/main.js`（MIT 许可，源码对应 `extensions/git/src/commands.ts`、`git.ts`、`repository.ts`、`actionButton.ts`）。中文文案取自 `ms-ceintl.vscode-language-pack-zh-hans` 的 `vscode.git.i18n.json`。只参照行为和文案，代码全部重写。

## Goals / Non-Goals

**Goals：**

- 快速选择框做成一个通用部件：调用方传入条目，**等待**用户的选择结果。这样多步流程（选起点 → 输入名字 → 签出 → 失败后询问 → 储藏后重试）可以按顺序写成一段异步代码，和 VS Code 的 `await showQuickPick` 结构一致。
- 每个分支操作对应的 git 命令和 VS Code 默认设置下执行的命令一致。
- 「合并 / 变基进行中」从仓库的文件状态读出来，不在应用里记录「刚才执行过合并」这类标记。

**Non-Goals：**

- 不做命令面板（⇧⌘P），快速选择框不注册成全局命令入口。
- 不改 vendor 组件的公开接口。如果实现时发现缺少某个能力，以补丁形式放进 `patches/`，并在任务中单独列出。实际只加了一处：`Command::search_icon(bool)`，默认开启、不影响原有用法，用来去掉 VS Code 快速选择框没有的放大镜图标（任务 6.3）。

## Decisions

### 1. 快速选择框：在 `Command` 外面包一层，过滤和排序由我们自己做

新建 `src/workbench/quick_input.rs`，提供两个异步入口：

```rust
// 返回用户确认的那一项；Esc、点外面或窗口失焦时返回 None
fn pick<T>(&mut self, options: PickOptions<T>, window, cx) -> oneshot::Receiver<Option<T>>
// 返回输入的文字；校验函数每次输入后调用一次
fn input(&mut self, options: InputOptions, window, cx) -> oneshot::Receiver<Option<String>>
```

`Workbench` 持有 `quick_input: Option<QuickInput>`，渲染时作为覆盖层画在根节点上（绝对定位，水平居中，宽度取 `min(窗口宽 × 0.62, 600)`），这一层同时接收「点击外部」的事件。打开新的会替换旧的，旧的发送端被丢弃，等待它的一方因此收到 None。

**为什么用 `filterable(false)`、自己过滤：** VS Code 的列表顺序会随输入变化。输入为空时，三个操作条目在最前；输入了文字后，匹配的引用在前，接着是分隔线，最后是操作条目。另外分组名要显示在该组第一行的右侧，而且分组里一项都不匹配时分组名也要跟着消失。`Command` 自带的过滤只会隐藏条目、不会调整顺序，也做不到「分组名跟在某一行上」。所以改为在 `on_query` 回调里重新生成整个条目列表：一组匹配的条目 = 名称不区分大小写地包含输入；分组名作为该组第一项的一部分画出来；组与组之间用 `Command::separator()` 隔开。

**为什么不直接用 `Command` 自带的 `CommandGroup` 标题：** 它会把组名画成单独的一行，而 VS Code 把组名画在该组第一行的右侧（见用户截图里的「分支」）。

**行的样式：** 用 `CommandItem::child` 自己画两行：第一行是图标、名称（匹配部分加粗 / 高亮，用 `StyledText::with_highlights`）、灰色说明，最右侧是可选的组名；第二行是灰色详情，超长截断。
**按键：** 上下键、循环、回车、Esc 由组件处理。⌘↑ / ⌘↓ / PageUp / PageDown 在我们的键位上下文里额外绑定，调用 `CommandState::set_selected_index`。
**加载中：** 先用空列表打开并调用 `set_loading(true)`；后台读出引用后再填入条目、关闭加载，期间已输入的文字照常参与过滤。

**输入框模式：** 用 `InputState`。它下面是提示行，提示行根据校验函数的返回值（`None` / `Info(text)` / `Error(text)`）切换样式；返回 `Error` 时回车不生效。点击外部不关闭（spec 要求，和 VS Code 的 `ignoreFocusOut` 一致）。

被否决的方案：
- 用 `Dialog` 包住 `Command`：Dialog 有遮罩，而且是在窗口中间弹出，和 VS Code 的位置不一样，测试里的焦点处理也更复杂。
- 用 `List` + `ListDelegate` 从头写：键盘、滚动、加载这些 `Command` 已经实现并且有测试，没有必要重写一遍。

### 2. 流程编排：一个操作 = 一段 `cx.spawn_in` 异步代码

每个入口（签出到、创建分支……）写成 `Workbench` 上的一个方法，在 `spawn_in` 里依次 `await` 选择结果、输入结果和 `window.prompt`，最后调用 `run_operation` 执行 git。例如「签出」：

```
ref = await pick(签出列表)          // None 就结束
result = await 执行签出(ref)
if result 是 DirtyWorkTree:
    choice = await prompt("签出会覆盖本地更改。", [储藏并签出, 迁移更改, 强制签出, 取消])
    执行对应的组合命令（储藏 + 签出 [+ 弹出储藏]，或 放弃全部 + 签出）
```

为此 `run_operation` 需要一个变体，能把错误交回给调用方。新增 `run_operation_with`，它返回一个 `Task<Result<(), GitError>>`，仍然占用同一个「执行中」位置、仍然在结束后刷新，但**不**自动弹出失败通知，由调用方处理自己认识的错误，其余错误再交给 `report_failure`。原来的 `run_operation` 改成在它上面「失败就通知」的一层薄包装，现有调用点不受影响。

被否决的方案：在 `report_failure` 里根据错误类型决定要不要弹确认框。那样会把「签出失败后该问什么」这种流程逻辑塞进一个通用的通知函数里，判断会越写越细。

### 3. git 层：错误按「git 自己给出的信号」分类

在 `GitError` 中新增：
- `DirtyWorkTree`：签出的 stderr 匹配 `Please,? commit your changes or stash them`。这和 VS Code 判断的依据相同，git 也没有提供更结构化的信号。
- `BranchNotFullyMerged`：`branch -d` 的 stderr 包含 `is not fully merged`。
- `Conflict`：`merge` / `rebase` / `stash pop` 失败后，用 `git ls-files -u` 检查是否有未合并的条目，有就算冲突。**这里不解析 stdout 里的 `CONFLICT` 字样**：仓库里实际存在未合并条目，比命令输出的文字更直接可靠，而且 rebase 和 stash pop 也能用同一种判断。

删除 `AmbiguousRemote`（多个远程时改为让用户选），`NoRemote` 的文案改为「仓库未配置任何要发布到的远程仓库。」。

新增的写命令（都在 `git/write.rs`，参数取自 VS Code 1.117）：

| 操作 | 命令 |
|---|---|
| 签出分支 / 标记 | `checkout -q <name>` |
| 签出远程分支并跟踪 | `checkout -q --track <remote>/<name>` |
| 分离签出 | `checkout -q --detach <commit>` |
| 创建分支 | `checkout -q -b <name> --no-track <start>` |
| 重命名 | `branch -m <name>` |
| 删除 / 强制删除 | `branch -d <name>` / `branch -D <name>` |
| 删除远程分支 | `push <remote> --delete <name>`（走 `remote_command`） |
| 合并 | `merge <ref>` |
| 变基 | `rebase <ref>` |
| 继续变基 | `rebase --continue`，环境变量 `GIT_EDITOR=true` |
| 中止 | `merge --abort` / `rebase --abort` |
| 储藏（含未跟踪） | `stash push --include-untracked` |
| 弹出储藏 | `stash pop` |
| 放弃全部（强制签出） | `reset --hard`，然后 `clean -fd` |
| 发布到指定远程 | `push --set-upstream <remote> <branch>`（已有，改为接受远程参数） |

弹出储藏不带 `--index`，与 VS Code「迁移更改」调用的 `stashPopLatest` 默认一致：改动回到工作区，原先的暂存状态不恢复。

### 4. 引用列表：一次 `for-each-ref` 读出全部

```
git for-each-ref --sort=-committerdate \
  --format=%(refname)%00%(objectname)%00%(*objectname)%00%(authorname)%00%(*authorname)%00%(committerdate:unix)%00%(*committerdate:unix)%00%(subject)%00%(*subject)%00%(upstream)%00%(upstream:track)
```

`*` 开头的字段用来取附注标记所指向的提交；`upstream` 用来判断「哪个本地分支跟踪了这个远程分支」；`upstream:track` 解析出 `[ahead N, behind M]`。和 VS Code 一样，使用 `%00` 分隔字段、按 `committerdate` 倒序排序。结果解析成 `Vec<Reference { kind: Local|Remote{remote}|Tag, name, commit, author, date, subject, upstream, ahead_behind }>`。这一步只在打开快速选择框时执行，不放进每次刷新：刷新很频繁，引用列表只有选择分支时才需要。

**性能：** 瓶颈在 git 遍历引用，几千个引用时也只需要几十毫秒，所以快速选择框先打开、再加载（决定 1），避免点击后界面没有反应。

### 5. 合并 / 变基进行中：作为 Snapshot 的一部分

`load_snapshot` 增加一步：用 `git rev-parse --git-path MERGE_HEAD --git-path rebase-merge --git-path rebase-apply` 取得这几个路径（兼容 worktree），检查文件是否存在，得到 `in_progress: Option<Merge | Rebase>`，并把它写进 `key`，状态变化时界面才会更新。路径每次刷新现取：多一个只需几毫秒的进程，换来 `Repository` 里不必多存一份缓存，也不用考虑缓存何时失效（实现时从「打开时缓存」改成这样）。

**为什么是这个信号：** `MERGE_HEAD`、`rebase-merge/`、`rebase-apply/` 就是 git 判断「合并或变基是否进行中」时所依据的文件，VS Code 也是读这几个文件。如果改为在应用里记录「刚刚执行过合并」，用户在终端里做的合并就读不到。

合并进行中、且提交信息输入框为空时，读取 `MERGE_MSG`，去掉以 `#` 开头的注释行后填进输入框（在刷新回调里判断「状态从 None 变为 Merge」时只填一次，避免覆盖用户已经输入的内容）。

### 6. 操作按钮：扩展现有的 `ActionButton` 枚举

在已有的 `Commit | Sync` 之外加上 `Continue(Progress) | Publish`。`ActionButton::of` 的判断顺序就是 spec 里列出的 1 → 5，每一步都直接读 `Snapshot`（`progress`、文件数、`head`、`upstream`），没有额外状态。点「继续」时：合并状态走现有的 `commit`；变基状态执行 `rebase --continue`。

### 7. 「…」菜单与动作

`main.rs` 新增动作：`Checkout`、`CheckoutDetached`、`CreateBranch`、`CreateBranchFrom`、`RenameBranch`、`DeleteBranch`、`DeleteRemoteBranch`、`Merge`、`Rebase`、`Publish`、`AbortMerge`、`AbortRebase`、`StageAll`、`UnstageAll`、`DiscardAll`。菜单项通过 `action_context(focus)` 派发这些动作，和现在的「拉取 / 推送」一样。「更改」子菜单的三项复用分组标题按钮背后的 `stage` / `unstage` / `discard` 方法。

### 8. 相对时间

自己实现 `relative_time(unix秒, 现在) -> String`，阈值与 VS Code 的 `fromNow` 相同：30 秒内为「刚刚」，之后依次是秒、分钟、小时、天、周、月（30 天）、年。文案用中文，例如「3 分钟前」。本机的中文语言包缺少这一组文案，VS Code 因此显示英文（用户截图里是「3 minutes ago」）。这里统一用中文，和其它界面保持一致。

### 9. 分支名规范化

照 VS Code 的 `sanitizeBranchName` 重新实现：去掉首尾空白，去掉开头的 `-`，然后把以下内容替换成 `-`：开头的 `.`、`/.`、`..`、`~ ^ : \ *`、空白、结尾的 `/`、`.lock`（在结尾，或后面跟 `/`）、结尾的 `.`、`[`、结尾的 `]`。是否重名用 `refs/heads` 下的分支名判断。

## Risks / Trade-offs

- [`Command` 在 `filterable(false)` 时，每次输入都会重建全部条目，引用很多时可能卡] → 条目只保存轻量数据，行内容由 `child` 闭包按需绘制；`Command` 本身对列表做了虚拟化。先实测上千个引用的情况，有问题再缓存过滤结果。
- [靠 stderr 文字判断 DirtyWorkTree：git 使用其它语言时匹配不到] → 所有 git 命令都在 `LC_ALL=C` 下执行（现有的 `base_command` 已经设置），输出固定为英文。
- [「迁移更改」中弹出储藏时发生冲突，会留下冲突文件，而储藏条目不会被删除] → 和 VS Code 行为一致。通知会说明冲突，冲突文件出现在「合并更改」组；储藏条目留着，用户可以在终端处理。
- [强制签出使用 `reset --hard` + `clean -fd`，会删除未跟踪文件] → 只在用户从确认框里选择「强制签出」后执行，确认框的文字与 VS Code 相同。
- [变基过程中如果 git 需要打开编辑器（如 `rebase --continue` 要求编辑提交信息），会卡住] → 设置 `GIT_EDITOR=true`，与 VS Code 相同。不支持交互式变基（不做的事里已列出）。
- [快速选择框打开期间仓库被外部改动（如在终端里切换了分支）] → 列表是打开那一刻的数据。确认后执行的是真实的 git 命令，失败就通知；刷新后状态正确。

## Migration Plan

不涉及数据迁移。`GitError::AmbiguousRemote` 被删除，`publish` 的签名改为接受远程名，调用点只有 `Workbench::push`，在同一个任务里一起改。

## Open Questions

- 快速选择框列表的最大可见高度：VS Code 用的是 `min(窗口高 × 0.4, 若干行)`。实现时对照截图取一个固定值（约 440 像素），不影响 spec 和任务划分。
