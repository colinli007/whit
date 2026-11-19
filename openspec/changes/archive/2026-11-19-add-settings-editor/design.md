## Context

动机见 proposal.md 的 Why。和实现相关的现状如下。

- **设置**（`src/settings.rs`）：
  - `Settings { file, values: Values }`。`main.rs` 启动时 `Settings::load()` 读一次，然后作为 `Workbench` 的字段传入。
  - 读写都走 `json_file`：读用 `serde_json` 严格解析，有一行注释就整体失败；写用 `to_vec_pretty` 整体覆盖。
  - 写入入口只有 `workbench.rs` 里提交、同步的询问框，以及 `scm_panel.rs` 读取 `ActionButton` 时用到的值。
- **编辑区**挂在 `Repository` 下：`editors: EditorGroup`、`buffers: HashMap<SharedString, OpenBuffer>`（键是仓库相对路径）、`hovered_tab`、`tab_scroll` 都是 `Repository` 的字段。`Workbench.repository` 为 `None` 时，`render` 整页画欢迎页（`welcome.rs`）。
- **文件监视**（`src/watcher.rs`）：`RepositoryWatcher` 递归监视仓库根目录和 git 目录，100 ms 聚合成 `Batch`。`cfg(test)` 下不启动真实监视器，测试直接喂 `Batch`。
- **活动栏**（`activity_bar.rs`）只有竖排一种形态。两个侧栏视图各自画标题行：`explorer_panel.rs:61` 是「EXPLORER」，`scm_panel.rs:329` 是「SOURCE CONTROL」。
- 单窗口应用；`gpui-kit` 提供 `Select`、`Checkbox`、`PopupMenu`、`Notification`、剪贴板等组件。

## Goals / Non-Goals

**Goals:**

- `settings.json` 是设置唯一的事实来源。设置页面、询问框、外部编辑器都只是它的读者或写者，界面状态永远从它推出来，不另存一份。
- 设置读取的开销为 O(1)：每帧渲染读设置，只是读一个全局里的 `Copy` 结构体。
- 编辑区归属窗口之后，标签的身份只由「它显示的东西」决定（文件的绝对路径、某仓库分组下的 diff、设置页面），与当前打开的是哪个仓库无关。

**Non-Goals:**

- 不做通用的设置注册系统（Zed 的 `Settings` trait + `register`）。只有 4 项设置，用一张静态表和一个结构体就够了；以后设置多到需要按模块注册时再演进。
- 不做多窗口之间的设置同步。现在是单窗口，全局 store 天然只有一份。

## Decisions

### 1. 全局 SettingsStore，取代 Workbench 的字段

`SettingsStore` 实现 `gpui::Global`，在 `main.rs` 中以 `cx.set_global` 安装，包含以下字段：

- `file: Option<PathBuf>`
- `values: Values`：当前生效的值，`Copy`。
- `configured: BTreeSet<&'static str>`：文件里出现了哪些已知键，用来显示已修改标记。
- `last_text: Option<String>`：上一次读到的文件原文。
- `error: Option<SharedString>`：当前的语法错误。
- `_watcher`：监视器，见第 3 节。

读写方式：

- **读**：`SettingsStore::global(cx).values()`。
- **订阅**：`Workbench` 和设置页面都用 `cx.observe_global::<SettingsStore>` 订阅，回调里只做 `cx.notify()`。
- **写**：`SettingsStore::update(cx, key, value) -> Result<(), WriteError>`，写完 `update_global` 触发重绘。

参考 Zed 的 `crates/settings/src/settings_store.rs`（`SettingsStore` 结构体在 154 行，`Settings::get_global` 在 104 行），是重写而非复制。我们不要 Zed 的类型注册表、多层合并和迁移，只保留「全局 + observe」这个通知方式。

**为什么不继续用 Workbench 的字段**：设置有三个写者（设置页面、询问框、文件监视），读者遍布侧栏、活动栏和设置页面。用字段的话，每个写者都要手动通知所有读者。用全局加 observe，「值变了」这个信号只有一个出口。

### 2. 读取：JSONC，按键容错

依赖 `jsonc-parser 0.34.0`（MIT，dprint），启用 `cst` 和 `serde` 两个特性。读取分两步：

1. **解析**：`parse_to_serde_value::<serde_json::Map<String, Value>>`，选项允许注释和尾逗号。
   - 解析失败时，设置 `error`；`values` 和 `configured` 保持不变（启动时则是默认值）；`last_text` 更新为新文本，同一份坏文件不会重复报错。
2. **逐项取值**：用设置项表中每一项的 `read: fn(&Value) -> Option<T>` 从 Map 里取值。取不到或类型不对时，这一项用默认值。只要键存在，就记入 `configured`。
   - 这是 VS Code 的行为：非法值被忽略，但仍然算「在当前作用域中配置了」，所以「重置此设置」能删掉它。

收到新的文件文本时，先与 `last_text` 比较，相同就直接返回。这样 Whit 自己写入后监视器回调回来的那一次没有开销。参考 Zed `settings_store.rs:968` `set_user_settings` 里对 `last_user_settings_content` 的同样处理。

`Values` 保留现有的 `#[serde(rename)]` 结构，但不再整体反序列化，改为由设置项表逐项填充；新增 `activity_bar_location: ActivityBarLocation` 枚举。

### 3. 监视：监视目录，按文件名过滤

`watcher.rs` 新增 `DirWatcher`，复用现有的 100 ms 聚合线程（`gather`）：

- `notify` 以非递归方式监视若干目录，可以动态 `watch(dir)` / `unwatch(dir)`。
- 发出的批次里是绝对路径。

监视目录而不是文件本身，是因为编辑器保存时常用「写临时文件再改名」，原文件的 inode 会变，直接监视文件会丢事件。Zed 也是这么做的，见 `crates/settings/src/settings_file.rs:202` `watch_config_dir`（重写）。

`SettingsStore` 持有一个 `DirWatcher`，监视 `~/Library/Application Support/Whit/`（启动时创建这个目录，但不创建文件）。批次里含有 `settings.json` 或 `rescan` 时，在后台读文件，回到主线程交给第 2 节的读取逻辑。同一目录里的 `recent.json` 等文件的事件，按文件名过滤掉。

`cfg(test)` 下不启动真实监视器（与 `Repository::watch` 一致，原因是确定性调度器不接受外部线程唤醒）。测试直接调用 `SettingsStore::reload(text, cx)` 模拟外部修改。

### 4. 写入：在磁盘当前文本上做最小编辑，同步完成

`update(key, value)` 的步骤：

1. **前置检查**：
   - `settings.json` 在编辑器里有未保存的修改 → 返回 `WriteError::Dirty`。由调用方查询，store 不知道编辑器的存在，见第 9 节。
2. **读出磁盘当前文本**：不用 `last_text`，这样即使监视器还没送达最新内容，也不会覆盖外部刚做的修改。
   - 解析失败 → 返回 `WriteError::Invalid`。
3. **编辑**：用 `CstRootNode::parse` 得到保留注释和格式的语法树，在 `object_value_or_set()` 上操作：
   - 值等于默认值 → `get(key)?.remove()`。
   - 键已存在 → `set_value`。
   - 否则 → `append`。
   - 文件不存在或为空时，直接生成 `{\n    "<key>": <value>\n}\n`（VS Code 新建 `settings.json` 用 4 个空格缩进）。已有文件的缩进由 jsonc-parser 根据已有的兄弟节点推断。
4. **原子写入**：先写临时文件，再改名替换（沿用 `json_file::write` 的做法，抽出一个写文本的版本）。
5. **生效**：用新文本立即走第 2 节的读取逻辑，所以写入后同一帧内就生效，不等监视器回调。

**为什么同步而不是像 Zed 那样放进异步队列**（`settings_store.rs:568` `update_settings_file_inner`）：

- Zed 走 `Fs` 抽象，要支持远程开发，所以写入必须异步。
- Whit 的设置文件只有几百字节，本地读改写在 1 ms 以内。
- 同步写入让「拒绝写入」可以立即反馈到控件上（复选框弹回原状态），连续两次修改也不存在交错问题。

只编辑改动的那个键，是 VS Code 和 Zed 的共同做法。Zed 通过比较 serde 值的差异算出文本编辑（`settings_store.rs:886` `edits_for_update_inner` 加上 `settings_json::update_value_in_json_text`，GPL，不使用）；我们的键都在顶层，直接按键名编辑语法树就够了。

### 5. 设置项表（照 Zed 的 SettingField）

每个设置项是一个 `SettingItem`：

| 字段 | 含义 |
|---|---|
| `id` | 设置 ID |
| `category` | 所属分类，`工作台/外观` 或 `扩展/Git` |
| `title` | 标题，`("Activity Bar", "Location")` 这样分成类别和名称两段 |
| `description` | 说明 |
| `kind` | `Bool`，或 `Enum(&[(value, description)])` |
| `read` / `write` | 函数指针，操作 `Values` |
| `default_json` | 默认值的 JSON |

整张表是一个 `static SETTINGS: &[SettingItem]`。store 的读取、写入、「复制设置为 JSON」、设置页面的渲染和搜索，都只遍历这张表。参考 Zed `crates/settings_ui/src/settings_ui.rs:114` `SettingField { pick, write }`（重写）。

搜索时：

- 页面创建时为每项建一次索引：ID、标题、说明拆成小写词。
- 查询按空格拆成词，每个词都要在该项的索引中找到以它为前缀的词。这是 Zed `settings_ui.rs:2340` 一带的精确匹配阶段；Zed 后面还有一个模糊匹配阶段，4 项设置用不上，不做。
- `@modified` 作为特殊词，改为检查 `configured`。

### 6. 编辑区提到窗口级

**挪动的字段**：以下字段从 `Repository` 挪到 `Workbench`：

- `editors: EditorGroup`
- `buffers: HashMap<Arc<Path>, OpenBuffer>`
- `hovered_tab`
- `tab_scroll`
- 新增 `settings_editor: Option<Entity<SettingsEditor>>`

`diff`、`shown_patch` 留在 `Repository`，因为 diff 标签只在有仓库时存在。

**`TabKey` 改为**：

- `File(Arc<Path>)`：绝对路径，经过 canonicalize，与监视器事件里的路径同源。
- `Diff { group, path }`：不变，路径相对当前仓库。
- `Settings`

**为什么文件标签用绝对路径作身份**：标签显示的是磁盘上的某个文件，磁盘路径是对它最精确的标识。仓库相对路径只是派生出来的视图，用于面包屑、资源管理器定位和 git 状态；而且编辑区不再属于某个仓库之后，相对路径本身就有歧义。以下派生都放在显示层：

- 仓库内：`path.strip_prefix(root)` 得到相对路径，用于定位、着色和面包屑。
- 仓库外：路径以 `~` 缩写显示。

**`FileBuffer`**：改为只认识绝对路径。显示名由 `Workbench` 按上面的规则计算。

**仓库监视批次**：`Repository::files_changed` 收到的 `Change::Worktree(rel)` 先 `root.join(rel)`，再查 `buffers`。

**关闭文件夹、切换仓库时要关的标签**：

- 所有 `Diff`；
- `File(p)` 中 `p.starts_with(root)` 的。

询问保存的范围也只限这些标签（`dirty_files` 加一个按根目录过滤的参数）。退出应用时询问全部。

**渲染**：

- 有仓库：活动栏（按位置）、侧栏、编辑区、状态栏。
- 无仓库：编辑区占满窗口。
- 编辑区没有标签时：有仓库显示提示文字，无仓库显示 `render_welcome`。这是 Zed 的做法，见 `crates/workspace/src/pane.rs:4672`，空 pane 在没有 worktree 时把 `WelcomePage` 画在占位区（重写）。

### 7. 仓库外文件的监视

`Workbench` 持有一个 `DirWatcher`（第 3 节），只监视「打开的仓库外文件」所在的目录：

- 打开这样的文件时 `watch(parent)`，该目录最后一个这样的标签关闭时 `unwatch`。
- 收到批次后，对其中命中 `buffers` 的路径调用现有的 `FileBuffer::disk_changed`，静默重载、冲突、删除的处理全部复用。

`settings.json` 因此会被两个监视器各看一次：store 的监视器负责更新设置值，这个监视器负责更新编辑器里的内容。两者职责不同，不合并。合并的话，store 就得知道编辑器的存在。FSEvents 对同一目录的两个流，开销可以忽略。

### 8. 活动栏的四种形态

`ActivityBarLocation { Default, Top, Bottom, Hidden }` 从 `SettingsStore` 读取，各形态的画法：

- **Default**：现有的竖排，底部加「管理」齿轮。齿轮用 `PopupMenu`，菜单项「设置」绑定 `OpenSettings` 动作，右侧显示 ⌘,。
- **Top / Bottom**：把两个侧栏视图各自画的标题行，抽成一个公共函数 `side_bar_title(label, actions)`。Top 时用横排图标代替 `label`（`.title.has-composite-bar>.title-label{display:none}`）；Bottom 时在侧栏底部加一行 35 像素高的图标行。尺寸取自 VS Code 1.117 的 `workbench.desktop.main.css`：
  - 图标格：高 35 像素，左右内边距 3 像素，图标 16 像素。
  - 当前项指示线：1 像素。Top 时在底部向上 2 像素处，Bottom 时在顶部向下 2 像素处，颜色 `activityBarTop.activeBorder`。
  - 当前项背景：`activityBarTop.activeBackground`。
  - 徽标：compact 样式，`top: 17px`，字号 9 像素。
- **Hidden**：不画活动栏。

不画「管理」齿轮的形态：VS Code 在 Top、Bottom 时要求开启自绘标题栏（它会把 `window.customTitleBarVisibility` 从 `never` 强制改为 `auto`），然后把齿轮放进标题栏。Whit 用 macOS 原生标题栏，所以这三种形态都不画齿轮，见 proposal 的「不做的事」。

新增动作：

| 动作 | 快捷键 | 菜单 | 行为 |
|---|---|---|---|
| `ShowExplorer` | ⇧⌘E | 「查看 → 资源管理器」 | 设置 `side_view` 并聚焦对应焦点句柄 |
| `ShowSourceControl` | ⌃⇧G | 「查看 → 源代码管理」 | 同上 |
| `OpenSettings` | ⌘, | 「Whit → 首选项 → 设置」 | 打开或切换到设置标签 |
| `OpenUserSettingsJson` | 无 | 无 | 设置页面的 `{}` 按钮和错误通知按钮触发 |

没有仓库时，`ShowExplorer` 和 `ShowSourceControl` 什么也不做。

### 9. 拒绝写入与错误通知

`SettingsStore::update` 不知道编辑器的存在，所以由调用方组合：`Workbench::write_setting(key, value)` 先检查 `buffers` 中 `settings.json` 的 `is_dirty()`，再调用 store。失败时弹通知，文案照 VS Code `configurationEditing` 的中文：

- `Dirty`：按钮「保存并重试」调用现有的 `save_file`，成功后重做这次写入；「打开设置」切换到该标签。
- `Invalid`：「打开设置」。

询问框里「不再显示」之类的写入失败时，只是记不住这次选择，提交或同步本身照常进行。

设置页面的控件状态始终从 store 读取，所以写入被拒时，下一帧复选框或下拉框自然回到原值，不需要回滚逻辑。

读取时的语法错误：`Workbench` 观察 store 的 `error` 从 `None` 变成 `Some` 时弹一次通知，并按通知 ID 记住它；`error` 回到 `None` 时用同一个 ID 关闭通知。这是对「错误出现」和「错误消失」这两个变化的直接反应，不轮询。

### 10. 设置页面视图

`SettingsEditor` 是一个独立的 Entity，`Workbench.settings_editor` 懒创建，关闭标签时丢弃。它包含：

- 搜索框：gpui-kit `InputState`。
- 筛选按钮：`PopupMenu`，菜单项「已更改」。
- 列表：普通的 `overflow_y_scroll` 容器，配合 `ScrollHandle`。只有 4 项，不需要 `uniform_list` 虚拟化。
  - 目录跳转用 `scroll_to_item`。
  - 目录高亮：每帧用 `bounds_for_item` 找第一个顶部在视口内的分类标题。
- 控件：
  - 枚举用 `Select`，宽 320 像素，高 26 像素。
  - 布尔用 `Checkbox`，18 像素。
- 已修改竖线：2 像素，`settings.modifiedItemIndicator`。
- 每项左侧的齿轮（宽 22 像素）：悬停才显示，点击弹出 `PopupMenu`；复制用 `cx.write_to_clipboard`。

尺寸取自 VS Code 1.117 `settings-editor` 的样式：

| 部位 | 值 |
|---|---|
| 设置项内边距 | `12px 14px 18px` |
| 布尔项下内边距 | 26 像素 |
| 分组标题 | `padding: 10px 10px 10px 15px` |
| 目录行高 | 22 像素 |
| 目录宽度 | 200 像素 |
| 隐藏目录的阈值 | `NARROW_TOTAL_WIDTH = 700` |

主题里没有的颜色，在 `theme::colors` 中按 VS Code 深色默认值补齐，沿用现有做法。

## Risks / Trade-offs

- **jsonc-parser 的 CST 编辑在边缘格式下的输出与 VS Code 不完全一致**（如尾逗号、单行对象）→ 用单元测试覆盖：空文件、只有注释、单行 `{ "a": 1 }`、尾逗号、2 空格缩进的旧文件、删除最后一个键。只要求结果能正确解析且注释仍在，不要求逐字节和 VS Code 相同。
- **编辑区挪到窗口级，改动面大**：`workbench.rs` 中所有 `repository.editors`、`repository.buffers` 的调用点都要改，已有的标签和保存测试都会受影响 → 先做一个纯结构挪动的提交，保持现有测试全部通过，再加设置相关的行为。任务分组按这个顺序。
- **两个监视器读同一个文件，会有两次读盘** → 文件只有几百字节；store 那一侧还有 `last_text` 比较，第二次读到相同内容时直接返回。
- **旧版 Whit 写入的 `settings.json` 里有等于默认值的键**（`"git.suggestSmartCommit": true`），打开设置页面时这几项会显示已修改标记 → 这与 VS Code 对「键存在」的判定一致，用户点「重置此设置」即可清掉。不做自动清理，因为清理也是在替用户改文件。
- **macOS 原生标题栏下，Top 和 Bottom 形态没有齿轮入口**，和 VS Code 不完全一致 → 已写进 proposal 的「不做的事」，⌘, 和菜单始终可用。

## Migration Plan

- 设置文件的位置和键名不变，旧文件直接可读。
- 测试中构造 `Workbench` 的地方，原来通过参数注入 `Settings::load_from(临时目录)`，改为在 `TestAppContext` 上 `set_global(SettingsStore::load_from(临时目录))`，`Workbench::new` 去掉 `settings` 参数。
- `json_file::write` 保留给 `recent.json` 使用；设置的写入改走第 4 节的文本写入。
