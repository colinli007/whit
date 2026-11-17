## 1. 项目骨架

- [x] 1.1 `cargo init --name diffview`，在 `Cargo.toml` 中固定 `gpui-kit = "=0.9.1"`（由它带入 gpui-pre 和 gpui-component，不单独写 `gpui-pre-*`），加入 `anyhow`、`log`、`env_logger`，开发依赖 `tempfile`；edition 2024；验证：`cargo check` 通过，`cargo tree -p diffview | grep gpui-pre` 中只出现一个版本
- [x] 1.2 `src/main.rs` 中用 `gpui_kit::application().run` → `gpui_kit::init` → `gpui_kit::open_window` 打开一个显示占位文字的窗口；验证：`cargo run` 能看到窗口
- [x] 1.3 添加 `.gitignore`（`target/`）、README（构建前提：Rust、Xcode Metal Toolchain、git），`Cargo.lock` 纳入版本控制（不被忽略；实际 git 提交等用户决定）；验证：`git status` 中没有 `target/`

## 2. git 数据层（`src/git.rs`，不依赖 gpui）

- [x] 2.1 实现统一的 git 命令构造：全局参数 `--no-pager --no-optional-locks -c core.fsmonitor=false -c core.quotepath=false`，以及在仓库根目录下执行；区分错误类型「找不到 git」「不是 git 仓库」「git 执行失败（附 stderr）」；验证：单元测试覆盖这三种错误
- [x] 2.2 实现 `repository_root(folder)`：调用 `git rev-parse --show-toplevel`；验证：测试中在临时仓库的子目录调用，返回仓库根目录
- [x] 2.3 实现已跟踪文件 patch：判断有无 HEAD（无 HEAD 时用空树 `4b825dc642cb6eb9a060e54bf8d69288fbee4904`），执行 `git diff <base> --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ -M --unified=1000000`；验证：测试覆盖修改、删除、重命名、暂存与未暂存叠加、无提交仓库
- [x] 2.4 实现未跟踪文件 patch（实施时修正为 intent-to-add 方案，见 design 第 4 节）：`git ls-files --others --exclude-standard -z` 列出后，在 index 副本上 `add --intent-to-add`，与已跟踪改动同一次 diff；验证：测试覆盖新文件、被忽略文件不出现、二进制新文件、用户暂存区不被改动
- [x] 2.5 实现 `load_worktree_patch(root) -> Result<String>`，按顺序拼接 2.3 与 2.4 的结果；测试中用 `gpui_kit::component::diff::DiffFile::parse` 解析，断言文件数量、路径、状态、增删行数与预期一致；验证：`cargo test` 通过
- [x] 2.6 测试用户配置不影响解析：临时仓库中设置 `diff.noprefix=true`、`diff.mnemonicPrefix=true`、`color.ui=always` 后，2.5 的解析结果不变；验证：`cargo test` 通过

## 3. 打开文件夹（`open-repository`）

- [x] 3.1 `src/repository_view.rs` 中建立 `RepositoryView` 实体，包含空状态（「打开文件夹」按钮 + 可选错误信息）和仓库状态两种界面；验证：不带参数启动时窗口中央显示按钮
- [x] 3.2 实现打开流程：通过 `cx.prompt_for_paths` 只选目录 → 后台调用 `repository_root` → 成功则切换为仓库状态并把窗口标题设为根目录文件夹名，失败则显示对应错误；取消选择时保持原状；验证：手动打开普通文件夹、仓库子目录、非仓库文件夹，分别符合 spec 场景
  - 自动化（`repository_view.rs` 测试）：`opening_a_subfolder_through_the_dialog_shows_the_whole_repository`（点按钮弹框、只选目录不多选、选子目录显示整个仓库）、`cancelling_the_dialog_keeps_the_open_repository`、`folder_outside_a_repository_explains_why`。窗口标题在测试平台读不到（`window_title` 仅 macOS 实现），已在真实应用截图中确认。
- [x] 3.3 注册菜单（应用菜单：关于、退出；文件：打开文件夹… cmd-O、刷新 cmd-R）及对应 action；验证：菜单项可见，快捷键可用
  - 自动化：cmd-O、cmd-R 走真实按键分发（见 3.2、4.5 的测试）；菜单项触发的是同一组 action。原生菜单栏已在真实应用截图中看到「diffview」「文件」。cmd-Q 与「关于」未做自动化。
- [x] 3.4 支持命令行参数：第一个位置参数作为文件夹路径直接打开；验证：`cargo run -- <仓库路径>` 启动后直接显示改动视图
- [x] 3.5 再次打开时替换当前仓库：丢弃旧的刷新任务和 `DiffState`；验证：手动先后打开两个仓库，只显示第二个
  - 自动化：`opening_another_folder_replaces_the_repository`。

## 4. 改动视图（`worktree-diff`）

- [x] 4.1 仓库状态下创建 `DiffState`（`DiffMode::Split`、`with_context_lines(Some(3))`），右栏渲染 `Diff::new(&state)`；验证：打开有改动的仓库，看到左右对照、词级高亮、折叠条可展开
- [x] 4.2 实现刷新：后台执行 `load_worktree_patch` 与 `DiffFile::parse`，回到界面线程调用 `set_files`；新任务替换旧任务字段；出错时显示错误信息；无改动时显示「没有未提交的改动」；验证：手动在干净仓库与有改动仓库中分别检查
- [x] 4.3 左栏文件列表：用虚拟列表渲染状态、路径（重命名显示 `原路径 → 新路径`）、`+N −M`，二进制标注；左右两栏用 `resizable` 分隔；验证：手动检查重命名与二进制文件的显示
- [x] 4.4 点击列表项：高亮该项并调用 `DiffState::scroll_to_file`；验证：点击后右侧滚到对应文件头
  - 自动化：`clicking_a_file_selects_it`、`selection_survives_a_refresh`。右侧实际滚动由组件的 `scroll_to_file` 完成，测试只断言了选中与调用，滚动位置未断言。
- [x] 4.5 刷新时机：打开时、cmd-R、`cx.observe_window_activation` 中窗口变为激活时；刷新后保留仍存在的选中项；验证：打开仓库后在终端修改文件，切回窗口后无需操作即可看到新改动；连续按 cmd-R 不出现旧结果覆盖新结果
  - 自动化：`refresh_shortcut_reloads_the_changes`、`returning_to_the_window_reloads_the_changes`（同时断言了启动后的第一次激活不会触发重复读取）。

## 5. 性能验收

- [x] 5.1 构造约 5 万行改动的测试仓库（例如复制一个中型项目后批量修改），用 release 构建测量从打开到首屏显示的时间；验证：不超过 1 秒、快速滚动无明显卡顿。如不达标，按 design「Risks」中的方案对超大文件改用 3 行上下文后重新测量，并在本任务里记录测量结果
  - 测量（2026-11-13，M4，release）：仓库 = Zed 的 editor + project 源码共约 27.6 万行，139 个文件增删各 25784 行，另有 300 个未跟踪文件和 1 个二进制文件；patch 约 11.7 MB（全文上下文）。进程启动到 diff 首次带数据渲染 0.49–0.63 秒（3 次），其中 git 读取 + 解析 0.24–0.32 秒、`set_files` 约 13 毫秒。初版为 1.4 秒以上，瓶颈是逐个未跟踪文件起进程（0.7 秒）和启动时重复读取，修正见 design 第 4、5 节。全文上下文本身不是瓶颈（0.11 秒 vs 3 行上下文 0.11 秒），无需降级。
  - 快速滚动是否卡顿需要人工确认（自动化环境无法发送滚动事件）。
- [x] 5.2 在含数百个未跟踪文件的仓库中测量刷新耗时并记录；验证：界面线程在刷新期间保持可操作
  - 同 5.1 的仓库（300 个未跟踪文件）：读取在后台线程，0.24–0.32 秒；界面线程上只有 `set_files` 约 13 毫秒；内容未变时连解析和 `set_files` 都跳过。

## 6. 打包

- [x] 6.1 安装 `cargo-packager`，在 `Cargo.toml` 中添加 `[package.metadata.packager]`（产品名、bundle id、图标占位、格式 `app` 与 `dmg`、ad-hoc 签名）；验证：`cargo packager --release` 产出 `.app` 和 `.dmg`
- [x] 6.2 把 `.app` 拷到「应用程序」目录启动，打开一个仓库完整走一遍；验证：功能与开发版一致，不出现「已损坏」提示
  - 2026-11-14 已拷到 `/Applications/diffview.app`（ad-hoc 签名）。用 `open` 启动：无「已损坏」等系统拦截，空状态正常；带仓库参数启动后左右对照正常。

## 7. 收尾

- [x] 7.1 `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`cargo test` 全部通过
- [x] 7.2 对照两份 spec 逐条手动验收所有场景，未通过的记录在本文件并修复
  - 2026-11-14 在真实窗口上自动验收（用户开启了屏幕录制与辅助功能，用 CGEvent 模拟鼠标、System Events 操作菜单和对话框，按窗口号截图核对）：
    - open-repository：菜单「打开文件夹…」弹出系统对话框（文件灰色不可选、按钮为「打开」），选 demo-repo 后窗口标题变为 demo-repo 并替换了原来的 big-repo；命令行参数打开、空状态、非仓库提示、空 PATH 下「找不到 git 命令，请先安装 git」均截图确认；取消对话框由界面测试覆盖。
    - worktree-diff：暂存与未暂存合并、未跟踪、被忽略、无提交仓库由 git 测试覆盖；无改动提示、重命名、二进制、行内词级高亮、只有新增时左侧留空均截图确认；拖选文本后 cmd-C 得到选中内容；点击折叠条展开到第 1 行；点击第 21 个文件后右侧滚到该文件头并高亮；菜单「刷新」与「切到 Finder 再切回」都读到新文件；约 5 万行改动仓库快速滚动 200 次滚轮事件期间画面持续更新、无空白，内存约 330MB。
    - 退出：菜单「退出」、cmd-Q、窗口关闭按钮都会结束进程；「关于」弹窗显示「diffview / 版本 0.1.0」。
    - 未在真实窗口单独验证：「连续触发刷新只采用最新结果」（由新任务替换旧任务的设计保证，界面测试覆盖了单次刷新）。
