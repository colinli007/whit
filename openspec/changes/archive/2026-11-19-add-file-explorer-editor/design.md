## Context

动机见 proposal.md 的 Why，要求见 `specs/`。与实现方式相关的现状：

- **工作台状态**：`src/workbench.rs` 中的 `Repository` 只有一个 `open: Option<OpenFile>` 和一个始终挂载的 `diff: Entity<DiffState>`，编辑区（`src/workbench/editor.rs`）围绕它渲染。之所以始终挂载，是因为 diff 组件在绘制时测量视口并请求重新布局，挂载时机不对会白屏，这是已经踩过的坑。
- **刷新**：`refresh()` 在后台运行 `git::load_snapshot`（`src/git/read.rs:127`），用 `refresh_task` 替换来丢弃过期的结果，并用 `Snapshot.key` 比较来跳过没有变化的情况。触发时机之一是窗口重新激活（`workbench.rs:255` 的 `left_window`）。所有 git 调用都带 `--no-optional-locks`（`src/git/mod.rs:117`），`git status` 不会回写 `.git/index`。
- **编辑器组件**：gpui-kit 0.9.1 自带 `EditorState`（`gpui-base` 中的 `InputBaseState<EditorMode>`）和 `Editor`，基于 rope，tree-sitter 高亮、行号、折叠、查找都有。它有两处不足：
  - 撤销栈 `UndoManager` 是 `pub(crate)`，不提供版本号。
  - 能把一批区间编辑作为一步撤销应用的 `replace_text_in_ranges` 也是 `pub(crate)`。公开的 `replace_all` 会把光标和滚动位置重置到开头。
- **树组件**：gpui-kit 的 `TreeState` 要一次给全整棵 `TreeItem` 树，行由 `ListItem` 渲染。
- **确认与通知**：确认对话框统一用 `window.prompt(PromptLevel, …)`（如 `workbench.rs:357`、`branches.rs:817`），即 macOS 原生对话框；错误用 `window.push_notification(Notification::error(…))`。
- **退出**：`lib.rs:125` 直接 `cx.quit()`，没有任何拦截。

参考来源：Zed 的 `crates/workspace/src/pane.rs`、`item.rs`，`crates/language/src/buffer.rs`，`crates/project/src/buffer_store.rs`，`crates/worktree/src/worktree.rs`，`crates/fs/src/fs.rs`、`fs_watcher.rs`，`crates/project_panel/src/project_panel.rs`（提交 `bb5e6df`，2026-11-18）。以下凡标「参考 Zed」的地方，都只借鉴机制并用自己的结构重写，不复制代码（Zed 这些 crate 是 GPL）。交互一律以 VS Code 1.117 为准。

## Goals / Non-Goals

**Goals:**

- 判断「是否有未保存修改」用编辑器撤销栈上的版本号（VS Code 的 alternativeVersionId 规则），不比较内容，也不从外面推测撤销和重做。
- 文件变化只有一个信号来源，就是文件监听。资源管理器、打开的文件、改动列表都由它驱动，删除「窗口激活时刷新」。
- 资源管理器的开销只和「已展开、可见」的部分有关，和仓库总文件数无关。

**Non-Goals:**

- 不抽象成可扩展的「标签插件」体系，标签只有两种。
- 每个 diff 标签不单独保存滚动位置（见 Risks）。
- 不实现 Zed 的协同编辑缓冲区（向量时钟、CRDT）。

## Decisions

### 1. 标签模型：enum + 一个标签组

```
EditorGroup {
    tabs:    Vec<Tab>,          // 显示顺序
    active:  Option<TabId>,
    preview: Option<TabId>,     // 最多一个
    mru:     Vec<TabId>,        // 最近使用顺序，激活时移到末尾
}
Tab { id: TabId, kind: TabKind }
enum TabKind {
    Diff { group: ChangeGroup, path: SharedString },
    File(Entity<FileBuffer>),
}
```

- **去重键**：文件标签用路径，diff 标签用（分组, 路径）。
- **插入位置**：新标签插在当前标签右侧。
- **预览标签**：以预览方式打开时，如果已有预览标签，就在它的位置替换。
- **关闭**：关闭当前标签后，从 `mru` 中取出最近使用的另一个标签来激活。

参考 Zed `pane.rs` 的 `items` / `preview_item_id` / `activation_history`（`:399-460`、`:1046`、`:1530`、`:2147`），这里重写为上面的结构。

**为什么不用 Zed 的 `Box<dyn ItemHandle>`**：Zed 的面板要容纳几十个 crate 里的各种视图，所以需要 trait 对象和 downcast。Whit 只有两种标签，范围是封闭的，用 enum 匹配让「diff 标签不能保存」「文件标签才显示行列」这类差异在类型上一目了然，也不需要按类型 downcast。

**diff 标签共用一个 DiffState**：保留现有那一个始终挂载的 `DiffState`，激活某个 diff 标签时，把该文件的 patch 设进去（沿用 `show_open` 中「patch 相同就不重设」的逻辑，保住滚动位置）。文件标签激活时 diff 组件仍挂载，只是被文件编辑器盖住，这样不会重新踩「挂载时机不对就白屏」的坑。

另一个选择是每个 diff 标签各有一个 `DiffState`。好处是切换标签时各自的滚动位置能保留；代价是多个 diff 组件同时挂载，每个都要处理白屏问题。这一期不做。

### 2. 文件缓冲区与未保存判定

```
FileBuffer {
    path:          SharedString,     // 相对仓库根
    content:       Content,          // Loading | Text { editor, format } | Unsupported | Failed
    saved_version: u64,              // 上次读取或保存时的编辑器版本
    synced:        Option<Stamp>,    // 文本上次与文件一致时的 (mtime, len)
    deleted:       bool,             // 文件现在是否已从磁盘上消失（标签加删除线）
}
is_dirty = editor.version() != saved_version
```

参考 Zed `buffer.rs` 的 `saved_version` / `saved_mtime` / `has_conflict`（`:111-140`、`:2534`、`:2556`），以及 `did_save` / `did_reload`（`:1653`、`:1777`）。

「文件现在在不在」和「上次同步时的文件戳」是两件事，所以分成 `deleted` 和 `synced` 两个字段，对应 Zed 的 `disk_state` 与 `saved_mtime`。规格要求文件在原路径重新出现时标题恢复正常，而有未保存修改时仍要在保存时报冲突：重新出现时只清掉 `deleted`，`synced` 保持旧值，保存时比较就会发现不一致。

不需要 Zed 那样的粘性 `has_conflict` 标记。冲突只在保存时用到，每次保存都会重新比较 `synced` 和磁盘上的文件戳，所以没有需要长期保存的冲突状态。

**版本号从哪来**：给 gpui-base 打补丁（vendor 到 `vendor/gpui-base`，补丁存为 `patches/gpui-base-undo-version.patch`），规则照 VS Code `textModel` 的 alternativeVersionId（VS Code 是 MIT 许可）：

- 每个撤销事务带一个版本号，取自单调递增的计数器。连续输入合并进栈顶事务时，栈顶事务换一个新版本号（内容变了，版本必须变）。
- **当前版本** = 撤销栈顶事务的版本号；撤销栈为空时，取「基础版本」。`set_value` 这类不进撤销栈的整体替换会更新基础版本。
- 撤销后，当前版本变为新的栈顶事务的版本；重做后，变为被重做事务的版本。
- 新增 `pub fn version(&self) -> u64`，以及 `pub fn break_undo_coalescing(&mut self)`。后者在保存时调用，保证保存后的输入是新的一步撤销，规格中「保存后继续输入」的场景依赖它。
- 同一补丁把 `replace_text_in_ranges` 暴露为 `pub fn apply_edits(…)`，给第 4 条的静默重载使用。

**放弃的方案**：

- **比较当前文本和保存时的文本**：这是自创的第三套规则。用户已明确不开发新规则。
- **从外面监听撤销、重做动作来推算版本**：这是在猜编辑器内部的状态，合并、IME 组合输入、多光标都会让推算出错。

撤销栈上限 1000 步：保存时的那一步被挤出栈后，就再也回不到「未修改」。这和 VS Code 一致，接受。

**预览转正**：订阅 `InputEvent::Change`，触发时如果该标签是预览标签就清除 `preview`。这对应 Zed 的 `handle_item_edit`（`pane.rs:1062`）。是否脏在渲染时由 `is_dirty` 计算，不另外存状态。

### 3. 读取与保存

- **读取**在后台线程进行：
  - 前 8 KB 内有 NUL 字节 → 判为 `Unsupported`；`std::str::from_utf8` 失败 → 也判为 `Unsupported`。
  - 否则照 Zed 的做法（`worktree.rs:7287`、`:7414`）：以第一个换行判断文件是 CRLF 还是 LF，并去掉开头的 BOM，记为 `TextFormat { crlf, bom }`；交给编辑器的文本统一用 LF。保存时把所有换行转为文件原有的形式，有 BOM 就加回去。
  - 起初的设想是原样交给编辑器、不做转换。实现时发现编辑器按回车总是插入 `\n`，在 CRLF 文件里新加的行会是 LF，违反「保留文件原有的换行符」。统一成 LF 之后，保存时连粘贴进来的 `\r\n` 也会一起按文件的形式写出（一个文件只有一种换行符，和 VS Code、Zed 一致）。
- **语言识别**：与 diff 视图用同一套按路径选择语言的规则，确保两边配色一致。这个函数在组件里原本是 `pub(crate)`，所以 vendor 的 gpui-component 多了一个小补丁（`patches/gpui-component-language-for-path.patch`），把 `diff::detected_language` 公开出来。
- **保存**：
  1. 先 `stat` 文件。如果文件存在，且 `(mtime, len)` 与 `synced` 不同，而这次又不是用户点了「覆盖」，就返回冲突，由工作台推送「文件内容较新」的通知（带「覆盖」按钮），不写入。文件已经不存在时直接写入，即重新创建。
  2. 否则在后台截断后原地写入（`File::create` 跟随符号链接，inode 和权限不变）。参考 Zed `fs.rs` 的 `RealFs::save`（`:1027`）：它也是原地写入，而不是写临时文件再改名。
  3. 写之前记下 `version`；写完后把 `saved_version` 设为这个值，`synced` 设为新的 `(mtime, len)`，清除 `deleted`，并调用 `break_undo_coalescing`。写入期间如果用户又输入了内容，保存后仍然是脏的，这和 Zed 的 `did_save` 一致。
- **保存后不主动刷新 git**：写文件本身会触发文件监听，改动列表由第 5 条那唯一的信号更新。

### 4. 外部修改的重载

文件监听报告某个打开文件的路径有变化时，先 `stat`：

- 文件不存在 → `deleted = true`，标签标题加删除线。
- `(mtime, len)` 与 `synced` 相同（自己保存产生的事件），或缓冲区有未保存修改 → 只清掉 `deleted`，不读文件。这一步在后台比较文件戳，所以自己保存后不会把文件再读一遍。
- 文件存在且有变化：
  - **没有未保存修改**：在后台读取新内容，用 `similar`（3.x，按行比较）算出编辑序列，通过 `apply_edits` 作为一步撤销应用。然后把 `saved_version` 设为应用后的版本，并更新 `synced`。光标所在行不在变化区间内时，光标保持在原处。如果读文件期间用户又做了编辑（版本变了），就不应用，交给保存时报冲突。
  - `apply_edits` 不能直接用组件内部的批量编辑：它会在每处编辑的末尾各放一个光标（为多光标输入设计）。补丁里的 `apply_edits` 先记下原选区，编辑后再按编辑结果平移回去。
  - **有未保存修改**：不重载。保存时由第 3 条的 `stat` 比较发现冲突。

参考 Zed `buffer.rs` 的 `reload_impl` / `apply_diff`（`:1685`、`:2378`）：它也是在后台算差异再以最小编辑应用，来保住光标。Zed 用的是自己的 diff 实现；我们用 `similar`，它已经作为间接依赖在依赖树里。

**放弃的方案**：`replace_all` 整体替换，会把光标和滚动位置重置到开头，CLI 每改一次，用户的位置就丢一次。

### 5. 文件监听

- 使用 `notify` 8.x（最新稳定版，9.0 仍是 rc 预发布版）的 `RecommendedWatcher`（macOS 上是 FSEvents），递归监听仓库根目录。如果 `git rev-parse --git-dir` 不在根目录下（例如链接的工作树），也监听那个目录。
- 事件进入一个通道，由后台任务合并：收到第一个事件后等待 100 ms，把这期间的所有路径作为一批处理。参考 Zed `fs_watcher.rs` 和 `worktree.rs` 的 `FS_WATCH_LATENCY = 100ms`。
- 每批中的路径按下表分类：

| 路径 | 改动列表 | 资源管理器 | 打开的文件 |
|---|---|---|---|
| `.git/` 下的 `HEAD`、`index`、`refs/**`、`packed-refs`、`MERGE_HEAD`、`rebase-*/**` 等 | 刷新 | 不处理 | 不处理 |
| `.git/` 下的 `objects/**`、`logs/**`、`*.lock` | 不处理 | 不处理 | 不处理 |
| 被忽略的路径（见第 6 条） | 不处理 | 它的父目录已展开时，重读父目录 | 是打开的文件时按第 4 条处理 |
| 其它工作区路径 | 刷新 | 它的父目录已展开时，重读父目录 | 是打开的文件时按第 4 条处理 |
| FSEvents 报告「需要重新扫描」 | 刷新 | 重读所有已展开的目录 | 检查所有打开的文件 |

- 「刷新」就是调用现有的 `refresh()`：它会丢弃过期结果，`key` 没变就不更新界面，所以一批事件触发多次刷新也没有额外代价。
- 删除 `left_window` 和窗口激活时的刷新，`dismiss_picker_on_blur` 保留。
- **不会自激**：`git status` 带 `--no-optional-locks`，不写 `.git/index`；Whit 自己的 git 写操作产生的事件，最多让刷新多跑一次，结果被 `key` 去重掉。
- **性能**：瓶颈是构建时 `target/` 下每秒成百上千个事件。这些事件属于「被忽略的路径」，如果父目录没展开就直接丢弃，不会触发 `git status`。分类时只需要和被忽略路径的前缀集合比较。

### 6. 被忽略路径的来源

在 `load_snapshot` 的 `git status` 参数中加上 `--ignored=matching`，`Snapshot` 新增 `ignored: Vec<String>`（目录以 `/` 结尾）。判断一个路径是否被忽略，就看它是否等于某一项或位于某一项之下。

实测（本仓库，`target/` 下有 27706 个文件）：

| 参数 | 输出行数 | 耗时 |
|---|---|---|
| `--ignored=matching -uall` | `target/` 只有 1 行 | 0.01 s，和不加时相同 |
| `--ignored=traditional -uall` | 逐个列出，27706 行 | 0.33 s |

所以选 `matching`。被忽略的路径和改动列表来自同一次 git 调用，和终端里的 git 判断完全一致，不用自己解析 `.gitignore`。

### 7. 资源管理器：扁平的可见行 + uniform_list

```
Explorer {
    children: HashMap<PathBuf, Vec<DirEntry>>,  // 只有读过的目录才有
    expanded: HashSet<PathBuf>,
    rows:     Vec<Row>,      // 由 children + expanded 深度优先展开得到
    selected: Option<PathBuf>,
}
```

- **加载**：展开目录时在后台对它 `read_dir`，然后排序：先目录、再文件，按名称自然排序且不区分大小写（用 `natord::compare_ignore_case`，不自己写比较函数）。
- **重建**：任何变化（展开、折叠、重读目录、新快照）都整表重建 `rows`。
- **渲染**：用 `uniform_list` 只渲染可见区间。

参考 Zed `project_panel.rs` 的 `update_visible_entries`（`:4605`）：它也是「已展开集合 + 扁平可见行，每次变化整表重建」，交给 `uniform_list`（`:7732`）。Zed 的 worktree 会后台全量扫描、遇到被忽略的目录才停（`worktree.rs:6314`），我们改为纯按需读取，和 VS Code 资源管理器一致。

**性能**：重建的代价与「已展开的条目数」成正比，与仓库总文件数无关；渲染的代价与可见行数成正比。即使展开 5000 个子条目的目录，重建也只是一次 Vec 遍历。

**为什么不用 gpui-kit 的 `TreeState`**：它要求一次给全整棵树，和按需加载冲突；行样式也要改成 VS Code 的缩进参考线、22 px 行高和 git 着色。

**git 着色**：每次得到新快照，就算一张 `HashMap<路径, 状态>`：

- 文件的状态：同时在「更改」组和「暂存的更改」组时，取「更改」组的。
- 目录的状态：遍历所有改动文件，向每一级祖先目录合并优先级最高的状态。

改动文件通常只有几十到几百个，这张表重建的开销可以忽略。Zed 用 SumTree 做区间聚合（`git_traversal.rs:67`），那是为了几十万条状态的场景，这里不需要。

**图标**：文件沿用 `file_icons::icon_for`。文件夹使用 vscode-icons 的默认文件夹图标和展开状态图标（`default_folder.svg` / `default_folder_opened.svg`），由 `scripts/file_icons.py` 一并提取。按文件夹名区分的专属图标留到以后。

**定位当前文件**：当前标签变化时，展开它的所有祖先目录（按需读取），选中该行，并滚动到可见范围。位于被忽略目录中的文件不展开祖先目录，对应 Zed 的 `reveal_entry(skip_ignored)`（`project_panel.rs:7252`）。

### 8. 活动栏与侧栏视图

`Workbench` 新增 `side_view: SideView { Explorer, SourceControl }`，侧栏根据它渲染对应的视图，两者共用同一个 `resizable_panel`，所以切换视图时宽度不变。打开仓库时设为 `SourceControl`。活动栏的源代码管理图标显示改动数量徽标，数量就是 `snapshot.files` 的长度。

### 9. 保存询问、退出和关闭窗口

- **关闭单个标签**：用 `window.prompt(PromptLevel::Warning, …)`，按钮为「保存」「不保存」「取消」，和现有的放弃更改确认对话框用同一种机制。
- **退出、关闭窗口、关闭文件夹、切换仓库**：先收集所有脏的标签，再统一调用 `confirm_discard_dirty() -> Task<bool>`。它在一个脏标签时问单文件对话框，多个时问合并对话框。只有得到 `true` 才继续原操作。
- **拦截退出**：`Quit` 不再在 `lib.rs` 里直接调用 `cx.quit()`，而是交给工作台，询问通过后才退出。
- **拦截关闭窗口**：用 GPUI 的 `window.on_window_should_close`，先返回 `false`，询问通过后再关闭窗口。

### 10. 快捷键

| 快捷键 | 动作 | 作用范围 |
|---|---|---|
| `cmd-s` | `Save` | 文件编辑器 |
| `cmd-w` | `CloseActiveTab` | 打开仓库后的工作台 |
| `space` | 以预览标签打开选中的文件 | 资源管理器 |
| `cmd-down` | 以普通标签打开选中的文件 | 资源管理器 |
| `left` / `right` | 折叠 / 展开 | 资源管理器 |

注册方式和 `lib.rs` 现有的 `KeyBinding` 写法一致。

## Risks / Trade-offs

- **[多 vendor 一个 crate，升级成本上升]**：gpui-base 也要 vendor 和打补丁。
  - 缓解：补丁存入 `patches/`，并在项目记忆的「gpui-kit diff 实测坑」里记一条：升级 gpui-kit 时要连同 gpui-base 补丁一起重新应用。补丁只改 `UndoManager` 和两个 `pub` 包装函数，大约 30 到 50 行。
- **[切换 diff 标签会丢失各自的滚动位置]**：VS Code 会为每个编辑器保存视图状态。
  - 缓解：本期接受。以后需要时，再给 diff 标签存滚动偏移，切换时恢复。
- **[大文件编辑性能]**（任务 10.1，已解决）：最初实测 50 MB、72 万行的 JSON 打开要 21 s，按键 110 ms。用 macOS `sample` 采样定位到两个原因：
  - **打开**：组件的 `EditorState` 默认开启自动换行（`soft_wrap` 默认为 true），`DisplayMap::on_text_changed` → `TextWrapper::update_wrapped` 会把每一行按字素拆分并排版测宽，占打开时间的 87%。VS Code 的 `editor.wordWrap` 默认就是 `off`（大文件还会强制关闭自动换行、高亮和折叠，`textModel.ts` 的 `LARGE_FILE_SIZE_THRESHOLD` 20 MB / `LARGE_FILE_LINE_COUNT_THRESHOLD` 30 万行）；Zed 则从不在首帧前排版全部行（`wrap_map.rs` 后台分批、主线程最多等 5 ms）。所以文件编辑器照 VS Code 的默认值关闭自动换行，规格「文件编辑」写明长行不自动换行。
  - **按键**：只是测试环境的假象。组件渲染时，在无障碍功能开启或 `test-support` 特性下，每帧用 `Rope::to_string()` 把全文交给 `aria_value`；测试正好开着 `test-support`。去掉这个条件重测，50 MB 文件每次按键只要 0～3 ms。
  - 修正后（release，格式化 JSON，`cargo test --release --lib large_file -- --ignored --nocapture`）：

  | 大小 | 行数 | 打开（修正前） | 打开（修正后） |
  |---|---|---|---|
  | 1 MB | 1.5 万 | 0.11 s | 0.017 s |
  | 5 MB | 7.4 万 | 0.50 s | 0.046 s |
  | 20 MB | 29 万 | 2.3 s | 0.19 s |
  | 50 MB | 72 万 | 21 s | 0.46 s |

  - 遗留：系统开启 VoiceOver 等辅助功能时，组件每帧仍会拷贝全文，大文件会卡；VS Code 只把视口附近约 10 行交给屏幕阅读器（`accessibilityPageSize`）。这是 gpui-base 的设计，本期不改，记在这里。
  - 无需大小上限：VS Code 桌面版要超过 1024 MB 才会提示「文件过大」（`workbench.editorLargeFileConfirmation`），Zed 的硬上限是 6 GB。

- **[原地写入不是原子的]**：写到一半时进程崩溃，文件会被截断。
  - 这和 Zed、VS Code 的默认行为一致，换来的是保留 inode、权限和符号链接。接受。
- **[FSEvents 在系统繁忙时会合并或丢弃事件]**：
  - 缓解：收到「需要重新扫描」标志时全量刷新（见第 5 条表格最后一行）。⌘R 手动刷新保留。
- **[静默重载时光标保持依赖 `apply_edits` 对选区的调整]**：
  - 缓解：gpui-base 和应用里各有一个测试，在光标前后分别插入行，检查光标保持在原来的文字上。
- **[编辑器自称适合约 5 万行以内的文本]**：这是 `EditorState::new` 文档里的说法，和任务 10.1 的大文件实测一起评估。
- **[无头测试中没有真实的 FSEvents]**：
  - 缓解：监听层只负责产出「一批路径」，分类和处理是一个纯函数加工作台方法。测试直接喂入合成的路径批次；另外单独写一个集成测试，验证真实 watcher 能收到事件。

## Migration Plan

- 发布与回滚都随应用版本走，没有持久数据格式的变化。回滚就是回退本变更的提交，并从 `[patch.crates-io]` 中移除 gpui-base。
- `src/workbench/tests.rs` 中依赖「单个 open」和「窗口激活刷新」的用例，改写为多标签和合成文件事件的版本；`tests/visual.rs` 的基准截图需要重新生成。
