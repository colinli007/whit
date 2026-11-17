## Context

新仓库，没有任何既有代码。动机见 proposal.md「Why」，行为要求见 `specs/open-repository`、`specs/worktree-diff`。

调研时确认的、会影响方案的事实：

- `gpui-component` 0.9.1 自带只读 diff 组件（`gpui_component::diff`，约 9000 行含测试），已经覆盖左右对照（`DiffMode::Split`）、词级行内高亮（`DiffInlineUnit::Word`，内部用 `similar`）、未改动行折叠展开（`with_context_lines`、`with_expansion_lines`）、虚拟滚动、选择复制、按文件跳转（`DiffState::scroll_to_file`）、二进制文件识别（`DiffFile::is_binary`）。
- 这个组件的输入是 unified patch 文本（`DiffFile::parse(&str)`；经 gpui-kit 引用时路径为 `gpui_kit::component::diff`），只展示 patch 里提供的行，不会自己补全上下文（源码注释：「unavailable context is never reconstructed」）。
- `gpui-component` 锁定依赖 `gpui-pre =0.3.12`（Zed gpui 的快照发布版），不是 crates.io 上的 `gpui` 0.2.2。所在仓库 longbridge/gpui-kit 推荐应用只依赖一个总包 `gpui-kit`：它锁定配套的 gpui-pre 各 crate（包括 `gpui-pre-platform`），并重新导出 gpui、gpui-base、gpui-component（README「Usage」一节）。
- Zed 的 git 读取全部通过子进程调用 `git`，2026-06 的提交 `c57de85528`（PR #53453）删掉了 libgit2，原因是 libgit2 不支持 reftable。它构造 git 命令时固定带 `-c core.fsmonitor=false`、`--no-optional-locks`、`--no-pager`，对不受信任的仓库再加 `--no-ext-diff` 等（`crates/git/src/repository.rs` 的命令构造函数，约 3800 行处）。

## Goals / Non-Goals

**Goals:**

- 改动列表和 diff 内容来自同一份数据，两者永远不会不一致。
- 读取改动全程在后台线程进行，界面线程只负责接收结果和渲染。
- 打开任意来源的文件夹都不会因为仓库里的配置去执行外部程序。

**Non-Goals:**

- 不自己实现 diff 算法和 diff 渲染，全部交给 git 和 gpui-component。
- 不做增量刷新（只刷新变化的文件）：第 1 期每次刷新都整体重新读取，先测量，确实慢了再优化。

## Decisions

### 1. diff 展示直接用 gpui-component 的 diff 组件

它已经实现了 spec 里几乎全部的展示要求，自己用 gpui 的列表重写一遍，至少是几千行代码。

备选方案：用 gpui 的 `uniform_list` 自己画，配合 `imara-diff` 自己算差异。放弃原因：工作量大一个数量级，并且和组件相比没有性能优势（组件同样是虚拟滚动）。

引入方式：只依赖总包 `gpui-kit = "=0.9.1"`，由它统一锁定 gpui-pre 全家和 gpui-component，不在 `Cargo.toml` 里逐个写 `gpui-pre-*`。这样不会出现几个 gpui-pre crate 版本互相不一致的情况。

代价：gpui 版本由 gpui-kit 决定，升级节奏跟着它走。

### 2. 数据来源是 git 输出的 patch，文件列表也从 patch 解析结果得出

组件吃的是 patch，而 `git diff` 输出的 patch 正是 git 自己的判断（遵守用户的 `diff.algorithm`、重命名检测等配置）。所以最直接的数据就是「git 的 patch → `DiffFile::parse`」，中间不需要自己读文件、自己算差异。

左侧列表直接从解析得到的 `Vec<DiffFile>` 生成（`path()`、`original_path()`、`status()`、`additions()`、`deletions()`、`is_binary()`），不再单独调用 `git status`。这样列表和 diff 来自同一份数据，不存在「列表里有、diff 里没有」的情况。

备选方案：先用 `git status --porcelain=v2` 拿列表，再逐个文件取 diff。放弃原因：两次读取之间文件可能被 CLI 改动，导致两份数据对不上，而且要多维护一套解析代码。

这推翻了此前讨论中「用常驻 `git cat-file --batch` 取文件内容 + `imara-diff` 计算差异」的设想。那个设想的前提是需要自己算差异，而组件直接接收 patch，这个前提不成立。

### 3. 用「全文上下文」的 patch，折叠交给组件

组件只能展开 patch 里有的行。要支持「一直展开到看见整个文件」，patch 里就必须带上全文。所以请求 git 时用 `--unified=<很大的数>`（实现时取 `1000000`），再由组件的 `with_context_lines(Some(3))` 默认只露出改动上下 3 行。

代价：patch 体积约等于所有改动文件的总大小，而不是只有改动部分。按「AI 一次改几十个文件」的典型场景，总量在几 MB 以内，解析和渲染都在后台完成或只渲染可见区域，可以接受。极端情况见「Risks」。

### 4. 读取改动的命令序列

所有命令都在仓库根目录下执行，公共参数参考 Zed 的做法：

- 全局参数：`--no-pager --no-optional-locks -c core.fsmonitor=false -c core.quotepath=false`
  - `--no-optional-locks`：读取时不去刷新 index 文件，避免和用户在终端里同时执行的 git 命令抢 `index.lock`。
  - `core.fsmonitor=false`：防止仓库配置借 fsmonitor 钩子执行任意命令。
- diff 参数：`--no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ -M --unified=1000000`
  - `--no-ext-diff`、`--no-textconv`：不执行仓库配置里的外部 diff 和 textconv 程序。
  - 显式指定前缀：覆盖用户可能设置的 `diff.noprefix` 和 `diff.mnemonicPrefix`，保证 patch 格式稳定可解析。

步骤：

1. 定位根目录：`git rev-parse --show-toplevel`。失败时按 stderr 区分「不是仓库」和其他错误；启动进程失败（找不到 git）单独提示。
2. 判断有没有 HEAD：`git rev-parse --verify --quiet HEAD`。没有提交时用空树对象 `4b825dc642cb6eb9a060e54bf8d69288fbee4904` 代替 HEAD。
3. 列出未跟踪文件：`git ls-files --others --exclude-standard -z`。
4. 没有未跟踪文件时，直接 `git diff <HEAD 或空树> <diff 参数>`，一次得到「暂存 + 未暂存」相对 HEAD 的最终结果。
5. 有未跟踪文件时，用 git 自带的 intent-to-add 让它们以「新增文件」出现在同一次 diff 里：把 index（路径取自 `git rev-parse --git-path index`，兼容 linked worktree）复制到临时目录，用 `GIT_INDEX_FILE` 指向这份副本，执行 `git --literal-pathspecs add --intent-to-add --pathspec-file-nul --pathspec-from-file <列表文件>`，再用同一份副本执行第 4 步的 diff。用户真正的 index 不被改动（有测试覆盖）。
6. 输出交给 `DiffFile::parse`。

实施时的修正（2026-11-13）：原计划对每个未跟踪文件执行一次 `git diff --no-index -- /dev/null <路径>`、最多 8 个并发。在 300 个未跟踪文件的仓库里实测这一步要 0.7 秒，是总耗时的大头（已跟踪部分的全文 diff 只要 0.11 秒）。intent-to-add 不管多少个未跟踪文件都只用固定几个进程，二进制判断、文件权限、`.gitattributes` 也都由 git 自己处理，比原风险预案「自己生成全部新增的 patch」更贴近 git 的结果。附带效果：「删掉已跟踪文件、再新建同内容的未跟踪文件」现在会被识别为重命名。

### 5. 状态结构与刷新

- `RepositoryView`（一个 gpui 实体）持有：仓库根目录、`Entity<DiffState>`、当前显示的 patch 文本、左侧选中的文件路径、当前错误信息、进行中的刷新任务。
- 刷新：在后台线程执行第 4 节的命令并调用 `DiffFile::parse`（纯计算，不依赖界面线程），完成后回到界面线程调用 `DiffState::set_files`。
- 内容没变就不替换：`set_files` 会重置滚动位置和已展开的折叠，所以后台拿到 patch 后先和当前显示的 patch 比较，完全相同就跳过解析和 `set_files`。「有没有变化」最准确的信号就是 patch 文本本身。
- 「只采用最新一次」：新的刷新任务直接替换字段里的旧任务，旧任务被丢弃即取消，不需要额外的序号判断。
- 刷新时机：
  - 打开仓库时；
  - 手动 cmd-R；
  - 窗口重新获得焦点时，用 `cx.observe_window_activation(window, ...)` 监听，并且只在窗口先失去过焦点、再重新获得时刷新（实施时修正：打开窗口后的第一次获得焦点不是「用户回来」，原写法会和打开流程同时各读一遍，实测让首屏变慢一倍）。之所以选这个时机：用户的典型操作是「切到终端让 CLI 改代码，再切回来看」，窗口重新获得焦点就是「用户回来看结果」最直接的信号，比定时轮询或监听文件变化都更准，而且没有额外开销。监听文件变化留到第 3 期。
- 保持选中：刷新后如果原先选中的路径仍在新文件列表里，就保留选中状态；不在了就清空。

### 6. 界面布局

- 启动走 gpui-kit 的统一入口：`gpui_kit::application().run(...)` → `gpui_kit::init(cx)` → `gpui_kit::open_window(...)`。`open_window` 会自动用组件库要求的 `Root` 包裹视图，不用自己包。代码里统一写 `use gpui_kit::*;`（即 gpui）和 `gpui_kit::component::...`，不直接引用 `gpui`、`gpui_component` 这些 crate 名（gpui-kit 源码注释里的约定，便于以后切换底层 gpui）；action 用 `gpui_kit::actions!` 宏定义。
- 两种状态：
  - 空状态：未打开仓库，或出现错误时，显示「打开文件夹」按钮，有错误时附带错误信息。
  - 仓库视图：左右两栏可拖动分隔线（gpui-component 的 `resizable`），左栏是文件列表（行数可能上千，用虚拟列表），右栏是 `Diff::new(&state)`，`DiffState` 设置为 `DiffMode::Split`、`with_context_lines(Some(3))`。
  - diff 开启自动换行 `soft_wrap(true)`（实施时补充）：不换行时组件让每栏宽度等于最长一行（`diff/mod.rs` 约 558 行），只要有一行超过半栏宽，右边就被推出窗口、要横向滚动才能看到。
  - diff 组件始终挂载，「正在读取 / 没有改动 / 出错」作为覆盖层盖在上面（实施时补充）：组件在绘制时测量可视宽度并 `cx.notify()` 请求重排，而 gpui 在绘制阶段收到的通知只记脏、不安排下一帧（`gpui-pre` `window.rs` 的 `invalidate_view`）。如果等数据到了才挂载组件，第一帧按 0 宽度排版，自动换行后整片空白，要等别的事件重绘才恢复（实测约一半概率）。先挂载空组件，让测量发生在数据到达之前即可避开。这是组件与 gpui 之间的问题，值得向 gpui-kit 反馈。
- 窗口标题：仓库根目录的文件夹名。
- 菜单：应用菜单（关于、退出），「文件」菜单（打开文件夹… cmd-O、刷新 cmd-R）。
- 文件夹选择用 gpui 的 `cx.prompt_for_paths`，参数设为只选目录、不可多选。

### 7. 代码组织

单 crate，按逻辑拆成三个文件，不拆得更细：

- `src/main.rs`：启动、菜单、快捷键、命令行参数、窗口创建。
- `src/git.rs`：第 4 节的全部 git 调用，输出 patch 文本或带原因的错误类型；不依赖 gpui，可单独测试。
- `src/repository_view.rs`：第 5、6 节的视图与状态。

### 8. 打包

用 `cargo-packager`，配置写在 `Cargo.toml` 的 `[package.metadata.packager]` 里，产出 `.app` 和 `.dmg`。签名用临时签名（ad-hoc，`-`），只保证本机可以运行。

备选方案：照搬 Zed 的 `script/bundle-mac`。放弃原因：它依赖 Zed 自己 fork 的 cargo-bundle，大部分逻辑服务于 CI 证书和多个二进制，我们都用不到。

## Risks / Trade-offs

- [gpui-pre 和 gpui-component 都比较新，API 可能变动] → 在 `Cargo.toml` 里固定版本，并提交 `Cargo.lock`；升级作为单独的改动来做。
- [全文上下文的 patch 在改动巨大的文件上体积大，例如重新生成的 lockfile] → 解析在后台进行，渲染只画可见行。实现后用「约 5 万行改动」的仓库按 spec 的 1 秒标准实测；不达标时，对超过阈值的单个文件改为默认 3 行上下文（这是第 4 节第 3 步里的参数，不影响整体结构）。
- [未跟踪文件很多（如漏写 `.gitignore` 的构建产物）时变慢] → 已改用 intent-to-add，进程数固定（见第 4 节）。
- [窗口每次回到前台都整体重读] → 新任务替换旧任务，不会叠加；内容没变时跳过解析和替换。第 3 期改用文件监听后再评估。
- [窗口尺寸变化时，自动换行的宽度可能慢一帧] → 原因同上面的「绘制阶段通知被丢弃」，拖动过程中连续重绘会自行纠正；彻底修复要在组件里改成下一帧再通知。
- [编译 gpui 需要 Xcode 的 Metal Toolchain] → 在 README 里写明构建前提。
