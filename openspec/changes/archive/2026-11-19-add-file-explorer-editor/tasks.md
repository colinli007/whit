## 1. 准备

- [x] 1.1 修改 `openspec/config.yaml` 的项目 context：把「编辑文件」从不做的列表中移除，写明本期做资源管理器、多标签编辑和文件监听；新建、重命名、删除文件，LSP，搜索替换，命令面板仍然不做。验证：`openspec validate add-file-explorer-editor --strict` 通过
- [x] 1.2 在 `Cargo.toml` 中加入 `notify`（最新的 8.x 稳定版）、`similar`（最新的 3.x）、`natord`，每个都附一行注释说明用途。验证：`cargo check` 通过，`Cargo.lock` 中这三个包的版本和预期一致
- [x] 1.3 把 `gpui-base 0.9.1` vendor 到 `vendor/gpui-base`，在 `[patch.crates-io]` 中加上它。验证：`cargo check` 通过，并且 `cargo tree -i gpui-base` 显示用的是本地路径

## 2. gpui-base 补丁：撤销版本号（design 第 2 条）

- [x] 2.1 在 `UndoManager` 中实现版本号：每个事务带版本；合并进栈顶事务时换新版本；撤销、重做后的当前版本取对应事务的版本；栈为空时取基础版本，`set_value` 更新基础版本。验证：单元测试覆盖以下几点：
  - 输入后撤销，版本回到原值。
  - 撤销后重做，版本回到撤销前的值。
  - 连续输入合并时，版本每次都变。
  - 输入 `x` 再退格删掉，版本不等于原值。
- [x] 2.2 暴露 `pub fn version()`、`pub fn break_undo_coalescing()`、`pub fn apply_edits()`（包装 `replace_text_in_ranges`）。验证：用 `TestAppContext` 写测试，覆盖以下两点：
  - 调用 `break_undo_coalescing` 后，接着输入，撤销时只撤销后输入的部分。
  - `apply_edits` 在光标前插入一行后，光标仍在原来的那行文字上。
- [x] 2.3 把补丁导出为 `patches/gpui-base-undo-version.patch`，并在 `Cargo.toml` 的 patch 注释中说明。验证：在全新解压的 gpui-base 0.9.1 上执行 `patch -p1 --dry-run` 能干净应用
- [x] 2.4 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过

## 3. git 读取：被忽略的路径（design 第 6 条）

- [x] 3.1 `load_snapshot` 的 `git status` 加上 `--ignored=matching`，解析 `!` 行，写入 `Snapshot.ignored`；新增 `Snapshot::is_ignored(path)`（判断路径等于某一项，或位于某一项之下）。验证：单元测试覆盖以下几点：
  - 忽略整个目录时只有一项。
  - 忽略单个文件。
  - 被忽略目录下的深层路径判为被忽略。
  - 名字相似但不同的路径（`target2/`）不判为被忽略。
  - 现有的 snapshot 测试全部通过。
- [x] 3.2 检查点：`cargo test` 通过；在本仓库（`target/` 下有几万个文件）里，`load_snapshot` 的耗时和改动前在同一量级（在测试中打印耗时）

## 4. 文件缓冲区：读取、保存、重载、未保存判定（design 第 2 到 4 条）

- [x] 4.1 新建 `src/workbench/file_buffer.rs`：`FileBuffer`、`Content`（`Text` / `Unsupported`）、`DiskState`；后台读取，判定二进制（前 8 KB 内有 NUL）和非 UTF-8；按路径选择语言，与 diff 视图用同一套规则。验证：测试覆盖以下几点：
  - PNG、GBK 文本得到 `Unsupported`。
  - UTF-8 文件得到 `Text`。
  - `.rs` 文件的语言是 rust。
- [x] 4.2 实现 `is_dirty` 和保存：先 `stat` 检查冲突；在后台原地写入；更新 `saved_version` 和 `disk`；调用 `break_undo_coalescing`。验证：测试覆盖以下几点：
  - 撤销到保存时的状态后，不再是脏的。
  - 手动改回原样，仍然是脏的。
  - 保存后继续输入，再撤销，回到干净状态。
  - CRLF 文件和带 BOM 的文件保存后字节完全一致。
  - 可执行权限保留。
  - 保存一个符号链接，写入的是链接指向的文件，链接本身仍在。
  - 没有写权限时返回错误。
  - 磁盘被外部修改后保存，得到冲突，文件没有被写入；选择覆盖后写入成功。
- [x] 4.3 实现外部修改的处理：事件对应的 `(mtime, len)` 和记录相同时忽略；文件不存在时标为 `Deleted`；干净的缓冲区用 `similar` 算出按行的编辑，通过 `apply_edits` 应用；脏的缓冲区不重载。验证：测试覆盖以下几点：
  - 自己保存后收到的事件不引起重载。
  - 干净缓冲区重载后内容等于磁盘内容、不是脏的，并且在光标之前插入的行不会让光标跳走。
  - 脏缓冲区内容保持不变。
  - 删除文件后为 `Deleted`；之后保存会重新创建文件，状态恢复为 `Present`。
- [x] 4.4 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过
  - `cargo test` 全部通过；clippy 只剩「尚未使用」告警（FileBuffer 与 `Snapshot::ignored` 的调用方在第 5、6 组），没有其它问题，`-D warnings` 的完整通过由 5.4 检查

## 5. 标签模型（design 第 1 条）

- [x] 5.1 新建 `src/workbench/editor_group.rs`：`EditorGroup`、`Tab`、`TabKind`，包含打开（去重、插在当前标签右侧、替换预览标签）、固定、激活（更新 mru）、关闭（激活 mru 中的上一个）。验证：单元测试覆盖 `file-editor` 规格中「多标签」「预览标签」「关闭标签」的全部场景
- [x] 5.2 用 `EditorGroup` 替换 `Repository` 中的 `open: Option<OpenFile>`，diff 标签共用现有的 `DiffState`，激活时再设置 patch（patch 没变就跳过）。SCM 中单击以预览方式打开，双击以普通方式打开；打开仓库时以预览方式打开第一个文件；刷新后，文件已不在原分组的 diff 标签关闭，如果关闭后没有任何标签，就打开第一个文件。验证：改写 `src/workbench/tests.rs` 中的相关用例，覆盖 `worktree-diff` 规格「点击文件打开 diff」「刷新」的全部场景（「从终端切回」改为通过合成文件事件触发），全部通过
- [x] 5.3 文件编辑时把预览标签转为普通标签（订阅 `InputEvent::Change`）。验证：无头测试中，单击打开 `a.rs` 后输入，再单击 `b.rs`，结果是两个标签
- [x] 5.4 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过
  - `cargo test` 144 个全部通过；clippy 只剩「尚未使用」：保存（6.4 接入）、外部修改处理（8.3 接入）、被忽略路径（9.x 接入），没有其它问题

## 6. 编辑区与状态栏的界面

- [x] 6.1 重写 `src/workbench/editor.rs` 的标签栏：
  - 多个标签，可以横向滚动，当前标签始终可见。
  - 当前标签和其它标签分别使用主题的活动、非活动标签配色。
  - 预览标签标题为斜体。
  - 脏标签显示圆点，悬停时变成关闭按钮。
  - 关闭按钮只在当前标签和悬停时显示。
  - 同名文件在标题后显示目录。
  - 鼠标中键关闭标签，双击标签固定。
  - diff 按钮只在 diff 标签上显示。

  验证：无头测试断言以上各项状态，覆盖 `file-editor`「标签的外观」和 `workbench-layout`「编辑区」的全部场景
- [x] 6.2 编辑区内容：当前是文件标签时显示 `Editor`（行号、折叠、查找），`Unsupported` 时显示规格中的提示文字；diff 组件保持挂载。验证：无头测试打开 `.rs` 文件，可以输入；打开 PNG 显示提示；从文件标签切回 diff 标签，diff 不会白屏（截图测试）
- [x] 6.3 状态栏右侧在文件标签下显示 `行 X，列 Y`。验证：无头测试移动光标后，文字随之更新；切到 diff 标签后不再显示
- [x] 6.4 加入 `Save`（cmd-s）和 `CloseActiveTab`（cmd-w）两个动作。SCM 文件行的「打开文件」改为以普通标签打开文件标签，并删除 `open_with_system`。验证：无头测试覆盖 ⌘S 写入磁盘、⌘W 关闭当前标签、「打开文件」打开的是文件标签；`file-editor`「保存」「从源代码管理打开文件」的场景全部通过
  - 「保存后出现在改动列表」依赖文件监听触发刷新，由 8.3 的测试覆盖；以普通方式打开的文件标签会把焦点移进编辑器（VS Code 行为，资源管理器双击场景同样需要）
- [x] 6.5 保存冲突的错误通知带「覆盖」按钮；保存失败时通知中包含文件名和原因。验证：无头测试制造冲突后，看到通知，点击「覆盖」后写入成功
- [x] 6.6 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过；运行 `cargo test --features visual-tests --test visual`，更新截图后人工比对标签栏和 VS Code 的截图
  - `cargo test` 全部通过；8 个截图场景通过（新增 19、20 号：文件标签、切回 diff 不白屏）；clippy 只剩「尚未使用」：外部修改处理（8.3 接入）、被忽略路径（9.x 接入）

## 7. 关闭前询问保存（design 第 9 条）

- [x] 7.1 关闭脏标签时弹出「保存 / 不保存 / 取消」对话框。验证：无头测试分别模拟三种回答，检查标签和磁盘的状态
- [x] 7.2 实现 `confirm_discard_dirty() -> Task<bool>`，一个脏标签时用单文件对话框，多个时用合并对话框；接入关闭文件夹和打开另一个文件夹。验证：覆盖 `open-repository` 增量规格的全部场景，以及「退出时有多个未保存文件」场景中的「全部保存」
- [x] 7.3 `Quit` 改为交给工作台处理，关闭窗口用 `on_window_should_close` 拦截。验证：无头测试中有脏标签时触发 `Quit`，选「取消」后应用不退出；人工在真实应用里点窗口的红色关闭按钮，确认会弹出询问
- [x] 7.4 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过
  - `cargo test` 154 个全部通过；7.3 的关闭按钮用 `VisualTestContext::simulate_close` 在无头测试中验证，没有操作真实窗口；关闭当前标签后焦点交给接着激活的标签（VS Code 行为，否则 ⌘W 会落在已关闭的编辑器上）；clippy 只剩第 8、9 组接入的「尚未使用」

## 8. 文件监听（design 第 5 条）

- [x] 8.1 新建 `src/watcher.rs`：用 `notify` 递归监听仓库根目录，git-dir 不在根目录下时也监听它；收到第一个事件后等 100 ms，把这期间的路径合并成一批发给工作台；FSEvents 要求重新扫描时发出「全部」标志。关闭文件夹或切换仓库时，丢弃监听器。验证：集成测试在临时仓库中写一个文件，500 ms 内收到包含该路径的批次；丢弃监听器后不再收到批次
- [x] 8.2 实现纯函数 `classify(batch, snapshot) -> Effects`（是否刷新改动列表、哪些目录要重读、哪些打开的文件要检查），规则按 design 第 5 条的表格。验证：单元测试逐行覆盖表格，包括 `.git/index.lock`、`.git/objects/..`、`target/debug/x`、`src/a.rs`、`.git/HEAD`，以及「需要重新扫描」
- [x] 8.3 工作台按 `Effects` 调用 `refresh()`、重读资源管理器目录、调用 4.3 的外部修改处理；删除 `left_window` 和窗口激活时的刷新。验证：无头测试喂入合成批次，覆盖 `file-watching` 规格的全部场景；构建场景下（连续 1000 个 `target/` 路径），`refresh` 一次都不被调用
- [x] 8.4 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过；启动应用，和终端并排放，在终端里 `echo x >> README.md`，1 秒内改动列表出现该文件，而且不需要切换窗口
  - 单元测试里不启动真实监听（确定性调度器拒绝外部线程唤醒），改为直接喂批次；真实 FSEvents 由 `watcher.rs` 的测试和截图场景 `changes_on_disk_show_up` 覆盖：离屏窗口里外部写入 `b.txt`，318 ms 后出现在改动列表，没有操作用户的真实窗口和终端

## 9. 活动栏与资源管理器（design 第 7、8 条）

- [x] 9.1 实现活动栏：48 px 宽，有两个图标，当前图标高亮并在左边缘显示竖线，源代码管理图标带改动数量徽标；加入 `side_view`，打开仓库时设为源代码管理；欢迎页不显示活动栏。验证：无头测试覆盖 `explorer`「活动栏与侧栏视图」和 `workbench-layout`「整体布局」的全部场景
- [x] 9.2 `scripts/file_icons.py` 增加提取 vscode-icons 的默认文件夹图标和展开状态图标，并新增 `file_icons::folder_icon(expanded)`。验证：重新运行脚本后，`assets/file-icons` 中出现这两个 SVG，`cargo test` 通过
- [x] 9.3 新建 `src/workbench/explorer.rs` 的数据部分：`children`、`expanded`、`rows` 的重建；后台 `read_dir`；用 `natord` 排序（目录在前）；排除 `.git` 等固定条目；读取失败时显示为空；全部折叠。验证：单元测试覆盖「条目的排列与排除」的全部场景，以及展开、折叠、全部折叠后 `rows` 的内容
- [x] 9.4 实现 git 着色表（文件取未暂存的状态优先，目录取子树中优先级最高的状态）以及被忽略条目变暗。验证：单元测试覆盖「git 状态着色」的两个场景，以及「被忽略的目录」场景
- [x] 9.5 实现资源管理器的界面：标题 `EXPLORER`；仓库名区域可以折叠，悬停时显示「全部折叠」；22 px 行高，每层缩进 8 px，带层级参考线；文件夹右侧显示圆点，文件右侧显示状态字母；用 `uniform_list` 渲染。验证：无头测试覆盖「资源管理器的结构」的全部场景；截图测试比对 VS Code
- [x] 9.6 交互：单击、双击、键盘（↑ ↓ ← → 空格 ⌘↓），以及当前标签变化时自动定位（被忽略目录中的文件不展开祖先目录）。验证：无头测试覆盖「单击、双击与键盘」和「定位当前文件」的全部场景
- [x] 9.7 性能：在临时目录中造一个有 5000 个子条目的文件夹，以及一个 `target/` 下有 20 万个文件的仓库。验证：打开仓库后资源管理器立即显示根目录内容；展开 5000 项的文件夹时，`rows` 重建和首帧渲染各自不超过 16 ms（在测试中计时）
  - 调试构建实测（`cargo test --lib explorer_scales -- --ignored --nocapture`）：带 20 万个被忽略文件的仓库打开到资源管理器显示 197 ms；展开 5000 项的文件夹，重建 0.45 ms、首帧 4.4 ms
- [x] 9.8 检查点：`cargo test` 和 `cargo clippy --all-targets -- -D warnings` 通过
  - `cargo test` 178 个通过（1 个按需运行的性能测试）；clippy 无告警；截图场景 22 号核对活动栏、git 着色、被忽略目录变暗、层级参考线

## 10. 收尾验收

- [x] 10.1 大文件实测：用一个 50 MB 的 JSON 文件测量打开耗时和输入延迟，结果记入本变更的 design.md 的 Risks。验证：如果不可接受，就停下来提出规格修改，不私自加阈值
  - 用户要求参照 Zed 和 VS Code 的做法。采样定位到打开慢的根因是组件默认开启自动换行、逐行测宽；照 VS Code `editor.wordWrap` 默认 off 关闭后，50 MB 打开从 21 s 降到 0.46 s。按键慢只是 `test-support` 每帧拷贝全文造成的测试假象，正式运行时 0～3 ms。不加大小上限；规格「文件编辑」补充了长行不自动换行和打开大文件的场景；详见 design.md 的 Risks
- [x] 10.2 在真实应用中走一遍完整流程：
  1. 在资源管理器中单击打开预览。
  2. 编辑后预览转正。
  3. ⌘S 保存，改动出现在 SCM。
  4. 终端修改同一文件，静默重载。
  5. 本地编辑时终端也修改，保存时提示冲突，然后覆盖。
  6. 删除文件，标题出现删除线。
  7. ⌘Q 时询问保存。

  验证：每一步截图，和规格场景逐条对照
- [x] 10.3 运行 `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo test --features visual-tests --test visual`，全部通过；`openspec validate add-file-explorer-editor --strict` 通过
  - `cargo test` 178 个通过（3 个按需运行的性能测试）；clippy 无告警；截图测试 11 个场景通过（新增 19～30 号截图，其中 `edit_save_and_follow_the_disk` 在离屏真实窗口中用真实文件监听走完 10.2 的全部步骤）；`openspec validate --strict` 通过
- [x] 10.4 更新项目记忆「gpui-kit diff 实测坑」：升级 gpui-kit 时要连同 `patches/gpui-base-undo-version.patch` 一起重新应用。验证：记忆文件里有这一条，`MEMORY.md` 的索引同步更新
