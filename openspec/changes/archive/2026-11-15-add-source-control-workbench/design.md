## Context

动机见 proposal.md「Why」，行为要求见 `specs/` 下四个文件。现状中与方案有关的事实：

- `src/git.rs` 目前只有读操作。所有命令走 `git_command()`：`stdin` 为空、`LC_ALL=C`、`GIT_TERMINAL_PROMPT=0`、`--no-optional-locks --no-pager -c core.fsmonitor=false -c core.quotepath=false`。改动用一次 `git diff <HEAD 或空树>` 读出（HEAD 对工作区），未跟踪文件在「复制出来的临时 index」上 `add --intent-to-add` 后一起出现在同一份 patch 里，用户的暂存区不被改动。
- `src/repository_view.rs`（约 700 行）一个实体包办空状态、文件列表、diff 和刷新。上一轮已改为：`Repository.files` 保存每个文件及其 patch 片段（`file_sections` 按 `diff --git` 切分），`DiffState` 里只放当前选中的一个文件；片段文本没变时不调用 `set_files`，从而保留滚动和选区。
- diff 组件来自 vendor 的 gpui-component（`vendor/gpui-component`，补丁在 `patches/`），已实现每栏裁切、同步横向滚动。组件提供 `DiffState::set_mode`（并排/内联）、`next_change`/`previous_change`。
- gpui-kit 0.9.1 中可直接用的部件：`Root`（`gpui_kit::open_window` 已自动包一层，所以通知可用）、`window.push_notification` + `Notification::error`、`window.prompt`（macOS 原生对话框，现有「关于」已在用）、`input::InputState`（多行 + 自动增高）、`button::DropdownButton`、`menu::PopupMenu`、`status_bar::StatusBar`、`resizable`、主题注册表 `ThemeRegistry::load_themes_from_str`（接受与 `theme/default-theme.json` 同格式的 JSON 主题集）、lucide 图标（`gpui_kit_assets::IconName` 共约 1800 个）。
- VS Code 的参照（MIT 许可，只参照行为与配色数值，代码全部重写）：配色取自 `extensions/theme-defaults/themes/dark_modern.json`；暂存、放弃、智能提交（无暂存时询问）、同步前确认等交互取自 `extensions/git/src/commands.ts`；状态字母与颜色取自 `extensions/git/src/repository.ts` 的资源装饰约定和颜色注册表中的 `gitDecoration.*` 默认值。

## Goals / Non-Goals

**Goals:**

- 读取路径保持「列表与 diff 来自同一份 patch」的原则：分组文件列表从各组 patch 的解析结果得出。
- 所有 git 命令（读和写）都在后台线程执行；写操作串行，一次只跑一个。
- 写操作的语义与用户在终端执行对应 git 命令完全一致（遵循用户的 git 配置、钩子、签名），应用不自行模拟 git 行为。
- 任何情况下 git 都不能因为等待交互输入而挂住。

**Non-Goals:**

- 不做增量刷新：每次刷新整体重读，沿用第 1 期的决定，测出慢再优化。
- 不引入 git 库（gix / git2），写操作同样调用本机 `git`。

## Decisions

### 1. 一次刷新读什么

每次刷新在后台依次执行：

1. `git status --porcelain=v2 --branch -z --untracked-files=all`：一次拿到分支名（`# branch.head`）、上游（`# branch.upstream`）、领先/落后数（`# branch.ab +A -B`，基于本地已有的上游引用，不联网）、冲突文件（`u` 行）、未跟踪文件（`?` 行）。
2. 「暂存的更改」：`git diff --cached <HEAD 或空树> --diff-filter=u <DIFF_ARGS>`，用真实 index，`--diff-filter=u`（小写）排除冲突文件。
3. 「更改」：沿用第 1 期的临时 index 方案，用第 1 步得到的未跟踪列表做 `add --intent-to-add`，再 `git diff --diff-filter=u <DIFF_ARGS>`（index 对工作区）。未跟踪文件因此以「新增」出现在这份 patch 里，再用未跟踪集合把它们的状态字母标成 U。
4. 「合并更改」：有冲突文件时，`git diff HEAD <DIFF_ARGS> -- <冲突路径…>`，展示工作区（带冲突标记）相对 HEAD 的内容。

为什么用 `status --porcelain=v2` 而不是继续单独调用 `ls-files --others`：分支、上游、领先落后、冲突、未跟踪这五样信息一次命令全拿到，而且是 git 定义好的机器可读格式；第 1 期的 `ls-files` 调用被它取代，命令数不增加。

为什么分组列表仍从 patch 得出、而不是从 `status` 得出：沿用第 1 期决定 2（列表与 diff 同源，不会出现列表里有、diff 里没有）。`status` 只提供 patch 里没有的信息（分支、计数、哪些是未跟踪、哪些冲突）。两条命令之间文件可能被改，最坏情况是某个未跟踪文件的字母暂时显示为 A，下一次刷新即纠正，可以接受。

数据模型：

```rust
enum ChangeGroup { Merge, Staged, Changes }        // 展示顺序即定义顺序
struct ChangedFile { group, file: DiffFile, patch: Arc<str>, letter: StatusLetter }
struct Snapshot { files: Vec<ChangedFile>, branch: BranchStatus }
```

当前打开的文件用 `(ChangeGroup, path)` 标识（同一文件可同时在两组）。上一轮「片段文本没变就不 `set_files`」的判断原样保留，只是键从 `path` 变成 `(group, path)`。整份结果相同时跳过解析的优化也保留（比较所有 patch 文本与 status 输出拼接后的结果）。

性能：瓶颈仍是全文上下文 patch 的体积（第 1 期决定 3）。部分暂存的文件会在两份 patch 里各出现一次全文，最坏情况体积翻倍；典型的 AI 改动场景下文件要么全暂存要么全未暂存，影响很小。新增的 `status` 命令在中等仓库约几十毫秒，且在后台执行。

### 2. 写操作的命令

写操作与读操作分开构造命令：去掉 `--no-optional-locks`（写操作本来就要拿锁）和 `core.fsmonitor=false`（让 git 按用户配置工作），保留 `LC_ALL=C`、`GIT_TERMINAL_PROMPT=0`、`stdin` 为空。各操作对应：

| 操作 | 命令 |
|---|---|
| 暂存 | `git add -A -- <路径…>`（`-A` 让删除也被暂存） |
| 取消暂存 | 有 HEAD：`git restore --staged -- <路径…>`；无 HEAD：`git rm --cached -r -q -- <路径…>` |
| 放弃（已跟踪） | `git restore --worktree -- <路径…>` |
| 放弃（未跟踪） | 删除文件（Rust `fs::remove_file`），不调用 `git clean`，避免误删目录里其他内容 |
| 提交 | `git commit -F -`（提交信息从 stdin 写入，避免命令行转义与长度问题）；修改上次提交加 `--amend`，信息为空时 `--amend --no-edit` |
| 无暂存时提交 | 用户确认后先 `git add -A`，再提交 |
| 拉取 | `git pull`（不加 `--rebase` 等参数，完全遵循用户配置） |
| 推送 | `git push`；无上游时 `git push -u <远程> <分支>` |
| 同步 | 拉取成功后推送 |
| 获取 | `git fetch` |

`commit` 需要从 stdin 写入信息，所以写操作的命令构造允许在提交时把 `stdin` 设为管道，其余仍为空。

备选方案：放弃更改用 `git checkout -- <路径>`。放弃原因：`restore` 是 git 2.23 之后的专用命令，语义更清楚（`--worktree` 明确只动工作区）。

### 3. 防止 git 等待交互输入

`GIT_TERMINAL_PROMPT=0` 只能阻止 git 自己在终端里问 HTTPS 用户名和密码，管不住 ssh：ssh 会直接打开 `/dev/tty` 询问私钥口令或主机指纹。从终端启动应用时进程有控制终端，ssh 就会挂在那里等输入。

做法：远程操作（拉取、推送、同步、获取）的子进程用 `pre_exec` 调用 `setsid()` 脱离控制终端。没有控制终端时，ssh 拿不到 `/dev/tty` 会立即失败，git 随之返回错误；用户已配置的 ssh-agent、钥匙串、GUI 凭据助手（如 Git Credential Manager）不受影响，照常工作。失败信息里包含 `Permission denied`、`could not read Username`、`Host key verification failed` 等时，界面提示「需要认证，请在终端中完成 git 认证配置后重试」并附原始输出。

备选方案：设置 `GIT_SSH_COMMAND="ssh -o BatchMode=yes"`。放弃原因：会覆盖用户自己的 `GIT_SSH_COMMAND` / `core.sshCommand` 配置。

依赖：`setsid` 需要 `libc`（已在锁文件中作为间接依赖，改为直接依赖）。

### 4. 视图结构：一个工作台实体，按区域拆文件

`RepositoryView` 改名为 `Workbench`，仍是窗口根视图，负责在「欢迎页」与「已打开仓库」两种状态之间切换。已打开仓库时它持有仓库状态（快照、当前打开的文件、`DiffState`、提交信息的 `InputState`、正在执行的写操作），渲染按区域拆到不同文件，都写成 `impl Workbench` 下的方法，事件用 `cx.listener` 直接调用 `Workbench` 自己的方法：

- `src/workbench.rs`：状态、刷新、写操作调度。
- `src/workbench/scm_panel.rs`：侧栏（标题行、输入框、提交按钮、分组列表）。
- `src/workbench/editor.rs`：标签栏、面包屑、diff。
- `src/workbench/status_bar.rs`：状态栏。
- `src/workbench/welcome.rs`：欢迎页（实施时放进 `workbench` 子模块，以便直接访问 `Workbench` 的私有字段）。
- `src/recent.rs`：最近项目的读写（不依赖 gpui，可单测）。
- `src/git.rs` → `src/git/mod.rs`（命令构造、错误）、`src/git/read.rs`（快照）、`src/git/write.rs`（写操作）。

为什么不把侧栏、编辑区拆成独立实体：它们读写的是同一份仓库状态（点侧栏要换编辑区内容，写操作要禁用侧栏按钮并更新状态栏）。拆成实体就要在实体之间传回调或互相订阅；放在同一个实体里按文件拆渲染代码，状态只有一份，也符合「单个组件能解决就不跨组件传值」。

写操作调度：`Workbench` 上一个 `operation: Option<RunningOperation>` 字段，非空时所有写操作入口不可用；`RunningOperation` 持有后台任务和操作种类（状态栏据此决定同步图标是否转动）。任务结束后清空字段、失败时推送通知，然后触发一次刷新。

### 5. 确认与报错用什么

- 确认（放弃更改、无暂存时提交、同步前确认、移除失效的最近项目）：用 `window.prompt`，即 macOS 原生对话框，与 VS Code 在 macOS 上的表现一致，也已有「关于」在用。
- 失败：`window.push_notification(Notification::error(...))`，右上角通知，内容为操作名 + git 输出。

### 6. 主题：打包一份 Dark Modern 主题 JSON

在 `assets/themes/dark-modern.json` 写一份 gpui-component 主题格式的主题集（只含一个深色主题），颜色值按 VS Code `dark_modern.json` 逐项对应（编辑区 `#1F1F1F`、侧栏/标签栏/状态栏 `#181818`、分隔线 `#2B2B2B`、文字 `#CCCCCC`、次要文字 `#9D9D9D`、按钮 `#0078D4`/悬停 `#026EC1`、输入框 `#313131`/边框 `#3C3C3C`、徽标 `#616161` 等）。启动时 `include_str!` 读入，`ThemeRegistry::load_themes_from_str` 注册，`Theme::change(ThemeMode::Dark)`，之后不监听系统外观变化。

状态字母颜色不进主题，直接按 VS Code 的 `gitDecoration.*` 默认深色值写成常量：修改 `#E2C08D`、新增 `#81B88B`、删除 `#C74E39`、未跟踪/重命名 `#73C991`、冲突 `#E4676B`。

diff 行底色原本由组件用主题的 success/danger 颜色乘固定透明度（行 0.12、词 0.3）绘制，任何主题色都调不出 VS Code 的效果。所以在 vendor 组件里新增 `Diff::change_colors(DiffChangeColors)`，由应用直接传入 VS Code 的 `diffEditor.insertedLineBackground #9BB95533`、`removedLineBackground #FF000033`、`insertedTextBackground #9CCC2C33`、`removedTextBackground #FF000033`；冲突片段仍用组件自己的冲突色。改动同步在 `patches/` 中。

### 7. 最近项目的存储

文件 `dirs::data_dir()/diffview/recent.json`（macOS 上即 `~/Library/Application Support/diffview/recent.json`），内容 `{"version":1,"folders":["/abs/path", …]}`，按最近打开排序、最多 50 项。写入先写同目录临时文件再 `rename`，避免写一半时崩溃把文件弄坏；读取失败（不存在、格式错）一律当作空列表并记日志，不影响启动。只在打开成功（定位到仓库根目录之后）时写入，记录的是仓库根目录。

文件很小（最多 50 行路径），启动时在界面线程同步读取即可，不值得为它开后台任务。

### 8. 提交信息输入框与快捷键

使用 `InputState` 的多行模式并开启自动增高（最多约 10 行后出现滚动条）。⌘Enter 通过给输入框所在区域设置独立的 key context（如 `ScmInput`）并绑定 `cmd-enter` → `Commit` 动作实现，不影响输入框内普通回车换行。占位文字中的分支名随快照更新。

## Risks / Trade-offs

- [应用会修改用户仓库，误操作代价高] → 放弃更改一律二次确认；未跟踪文件的删除确认文案明确写「永久删除、无法恢复」；所有写操作串行执行、执行中禁用入口，避免连点造成重复提交。
- [提交时会执行仓库里的 git 钩子和签名程序] → 与 VS Code、终端行为一致，属预期；第 1 期为「打开任意文件夹不执行仓库配置的程序」设的防护（`--no-ext-diff`、`--no-textconv`、`core.fsmonitor=false`）只保留在读路径上，打开和浏览仍不会执行仓库里的程序，只有用户主动提交、拉取时才会。
- [冲突条目混进两份 patch] → 实测（`conflicted_files_are_merge_changes`）：`git diff --cached --diff-filter=u` 不再输出冲突条目；但暂存区对工作区的 `git diff` 不论过滤参数都会为冲突文件输出组合 diff（`diff --cc`），解析器不支持。处理方式：解析「更改」组时丢弃以 `diff --cc ` 开头的文件片段，冲突文件只从「合并更改」组读取。
- [部分暂存文件的 patch 体积翻倍] → 见决定 1 的性能说明；超大仓库再测量。
- [GUI 凭据助手（如 Git Credential Manager）可能弹出它自己的登录窗口] → 这是用户主动安装的配置，属预期行为，不拦截。
- [diff 底色与 VS Code 有色差] → 决定 6 的两步方案，必要时改 vendor 组件。
- [欢迎页、侧栏的像素级还原] → 以用户提供的 VS Code 截图为准逐项对照，实施后用真实窗口截图比对。

## Migration Plan

本机桌面应用，无数据迁移。`recent.json` 首次运行时不存在，视为空列表。旧的规格（暂存/未暂存合并展示、连续视图）在本变更归档时被 delta 替换。回滚即回退代码，`recent.json` 留在原处无副作用。
