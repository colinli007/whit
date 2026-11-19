## Why

Whit 已经有 3 个设置：`git.enableSmartCommit`、`git.suggestSmartCommit`、`git.confirmSync`。它们只能在提交或同步时弹出的「始终 / 从不 / 不再显示」里写入，写进去之后，界面上没有任何地方能改回来。用户还希望像自己的 VS Code 那样，把活动栏放到侧栏顶部（`workbench.activityBar.location: top`）。这两件事都需要一个和 VS Code 一样的设置页面。

另外，现在读写 `settings.json` 的方式和 VS Code、Zed 都不一样：

- 读：用严格 JSON 解析，文件里有一行注释，整个文件就读不出来，全部回到默认值。
- 写：整个结构体覆盖写入，用户手写的其它键和注释都会丢失。
- 外部用编辑器改了这个文件，Whit 不知道。

这些都要一并改成和 VS Code 一致的行为。

## What Changes

- **设置页面**（VS Code 1.117 的设置编辑器）：
  - 以「设置」标签打开，入口有 ⌘,、菜单「Whit → 首选项 → 设置」，以及活动栏底部的「管理」齿轮。
  - 页面结构：搜索框（显示「找到 N 个设置」，支持 `@modified` 过滤），「用户」页签，左侧目录（「工作台 > 外观」「扩展 > Git」），设置列表。
  - 列表每项有标题、中文说明和控件（下拉框或复选框）。改过的项左侧显示竖线；悬停时显示齿轮，菜单里有「重置此设置」「复制设置 id」「将设置复制为 JSON 文本」。
  - 右上角的 `{}` 按钮用编辑器打开 `settings.json`。
  - 只列出 Whit 真正生效的 4 个设置：3 个 git 设置，加上新增的 `workbench.activityBar.location`。
- **活动栏位置**：新增设置 `workbench.activityBar.location`，取值与 VS Code 1.117 相同：
  - `default`：侧栏左边竖排，即现在的样子。
  - `top`：图标横排在侧栏标题行，取代标题文字。
  - `bottom`：图标横排在侧栏底部。
  - `hidden`：不显示活动栏。
- **切换侧栏视图的快捷键**：新增 VS Code 的 ⇧⌘E（资源管理器）和 ⌃⇧G（源代码管理），以及「查看」菜单里的对应项。活动栏隐藏后，用户仍能切换视图。
- **settings.json 的读写**（照 VS Code / Zed）：
  - 读：允许注释和尾逗号（JSONC）。
  - 写：只改动变的那一个键，文件里其它内容原样保留。选回默认值就删掉这个键。
  - 监视：监视文件变化，外部修改立即生效；文件写坏时，继续用上一次读对的值，并弹出提示。
  - 设置页面写入时，如果 `settings.json` 在编辑器里有未保存的修改，或者文件有语法错误，拒绝写入并提示，文案照 VS Code。
- **编辑区提到窗口级**（**BREAKING**，内部结构）：
  - 标签组原来属于打开的仓库，现在属于窗口。没打开仓库时也有编辑区。
  - 欢迎页变成「没有标签时编辑区显示的内容」。因此没打开仓库时也能打开设置标签；关掉设置标签，欢迎页内容又回来。
  - 关闭文件夹或切换仓库时，只关闭该仓库的文件标签和 diff 标签。设置标签和仓库外的文件标签（如 `settings.json`）保留。
- **编辑器可以打开仓库外的文件**：文件标签改为按绝对路径区分，以便打开 `settings.json`。仓库外文件的悬停提示和面包屑显示以 `~` 开头的完整路径。

## 不做的事

- 「工作区」页签和工作区设置（仓库里的 `.vscode/settings.json`）。它需要用户设置与工作区设置分层合并、监视该文件、「在工作区中也进行了修改」提示等一整套逻辑，以后单独立变更。本次只做「用户」页签。
- 把其它写死的行为做成设置（编辑器字号、自动换行、预览标签开关、`files.exclude` 等）。以后某个行为需要可配置时，再逐个加入设置。
- `@modified` 以外的搜索过滤词（`@feature:`、`@tag:` 等），以及搜索时的目录筛选和计数。
- 自定义标题栏。VS Code 在活动栏为 `top`、`bottom`、`hidden` 时，把「管理」齿轮放进它自绘的标题栏；Whit 使用 macOS 原生标题栏，所以这三种位置下不显示齿轮，入口只有 ⌘, 和菜单。
- 右键活动栏弹出的「活动栏位置」菜单、切换活动栏可见性的命令、紧凑活动栏（`workbench.activityBar.compact`）。
- 设置同步、配置文件（profile）、导入 VS Code 设置、命令面板。
- 键盘快捷方式编辑器。

## Capabilities

### New Capabilities

- `settings`：`settings.json` 的位置、格式（JSONC）与读取；设置项的定义与默认值；写入规则（只改动变的键、默认值删键、保留注释）；外部修改实时生效；文件损坏和拒绝写入时的提示。
- `settings-editor`：设置页面的入口、标签行为、页面结构（搜索、页签、目录、列表、`{}` 按钮）、每项的显示和控件、已修改标记与齿轮菜单、`@modified` 过滤。

### Modified Capabilities

- `workbench-layout`：「整体布局」中活动栏的位置改由设置决定；「编辑区」改为窗口级，没打开仓库时也存在，没有标签时显示欢迎页内容，并新增设置标签。
- `explorer`：「活动栏与侧栏视图」新增四种位置的外观、底部「管理」齿轮，以及 ⇧⌘E、⌃⇧G 和「查看」菜单。
- `file-editor`：「多标签」新增设置标签这一种，文件标签改为按绝对路径区分，可以打开仓库外的文件；「关闭标签」中关闭最后一个标签后的显示，按有无打开仓库区分。
- `open-repository`：「未打开时的空状态」改为编辑区里的欢迎页内容；「再次打开会替换当前仓库」和「关闭文件夹」只关闭该仓库的标签，设置标签和仓库外的文件标签保留。

## Impact

- **代码**：
  - `src/settings.rs`：重写为全局 `SettingsStore`，负责 JSONC 读取、最小文本编辑写入和设置项表。
  - 新增 `src/settings_watcher.rs` 或复用 `src/watcher.rs`，监视设置目录。
  - 新增 `src/workbench/settings_editor.rs`：设置页面视图。
  - `src/workbench.rs`：`EditorGroup`、`buffers`、标签悬停和滚动状态从 `Repository` 挪到 `Workbench`；渲染改为「活动栏（按位置）+ 侧栏 + 窗口级编辑区」，以及「无仓库时只有编辑区」。
  - `src/workbench/editor_group.rs`：`TabKey::File` 改为绝对路径，新增 `TabKey::Settings`。
  - `src/workbench/editor.rs`、`src/workbench/welcome.rs`：没有标签时显示欢迎页内容。
  - `src/workbench/activity_bar.rs`：竖排和横排两种形态，以及「管理」齿轮。
  - `src/workbench/scm_panel.rs`：读设置改为读全局 `SettingsStore`。
  - `src/lib.rs`：新增动作「打开设置」「打开用户设置(JSON)」「显示资源管理器」「显示源代码管理」，新增快捷键 ⌘,、⇧⌘E、⌃⇧G，新增菜单「首选项」和「查看」。
  - `src/main.rs`：启动时初始化全局 `SettingsStore`。
- **依赖**：新增 `jsonc-parser 0.34.0`（MIT，dprint 出品），启用 `cst` 和 `serde` 两个特性。它能读取带注释的 JSON，并在保留注释和格式的前提下编辑。不用 Zed 的 `settings_json`，因为它是 GPL。
- **测试**：已有测试通过 `Workbench::new` 注入 `Settings`，要改为设置全局 `SettingsStore`，仍然只用临时目录，不写用户真实的设置文件。
- **数据**：`~/Library/Application Support/Whit/settings.json` 的位置和键名都不变，旧文件可以直接读取。
