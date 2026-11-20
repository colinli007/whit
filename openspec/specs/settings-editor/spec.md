# settings-editor Specification

## Purpose

按 VS Code 1.117 设置编辑器的布局与交互，在编辑区以「设置」标签列出 Whit 的全部设置，可搜索、可修改、可重置，并能跳到 `settings.json` 直接编辑。

## Requirements

### Requirement: 打开设置

系统 SHALL 提供以下入口打开设置页面：

- 快捷键 ⌘,；
- 菜单「Whit → 首选项 → 设置」；
- 活动栏位置为 `default` 时，活动栏底部的「管理」齿轮：点击后弹出菜单，其中有「设置」（右侧显示 ⌘,）。

设置页面 SHALL 以编辑区中的标签打开：标题「设置」，带设置图标，以普通标签（非预览）打开。整个窗口 SHALL 最多一个设置标签，再次打开时切换到已有的标签。打开后焦点 SHALL 在搜索框。未打开仓库时，SHALL 也能打开设置页面（见 `workbench-layout` 的「编辑区」）。

#### Scenario: 用快捷键打开

- **WHEN** 已打开仓库，当前标签是 `a.rs`，用户按 ⌘,
- **THEN** `a.rs` 右侧新增「设置」标签并成为当前标签，光标在搜索框中

#### Scenario: 不重复打开

- **WHEN** 已有「设置」标签但当前标签是 `a.rs`，用户点击齿轮菜单中的「设置」
- **THEN** 切换到已有的「设置」标签，标签数量不变

#### Scenario: 欢迎页上打开

- **WHEN** 未打开仓库，用户按 ⌘,
- **THEN** 编辑区出现「设置」标签和设置页面；关闭该标签后，编辑区重新显示欢迎页内容

### Requirement: 页面结构

设置页面 SHALL 按 VS Code 1.117 的结构从上到下排列。尺寸和颜色取自 VS Code 1.117 的设置编辑器样式，颜色按当前主题。

- **标题区**（左右各留 24 像素边距，最大宽度 1400 像素，居中）：
  - 搜索框，占位文字「搜索设置」，右端依次是结果计数、清除按钮和筛选按钮。
  - 搜索框下方是页签行，只有一个「用户」页签，处于选中状态，下方有一条分隔线。
- **主体**：
  - 左侧是目录，默认宽 200 像素；右侧是设置列表。编辑区宽度小于 700 像素时，SHALL 隐藏目录。
  - 目录项：「工作台」下有「外观」，「扩展」下有「Git」。当前分类以粗体显示。
  - 点击目录项 SHALL 把列表滚动到该分类；滚动列表时，目录 SHALL 高亮当前可见的分类。
- **标签栏右侧**：设置标签为当前标签时，标签栏右侧 SHALL 显示「打开设置(JSON)」按钮（VS Code 的 `go-to-file` codicon），点击后在编辑器中打开 `settings.json`，规则见 `file-editor` 的「多标签」。文件不存在时，SHALL 先以内容 `{` 换行 `}` 创建该文件再打开（VS Code 的 `createIfNotExists`），空对象不改变任何设置。

#### Scenario: 目录跳转

- **WHEN** 用户点击目录中的「Git」
- **THEN** 列表滚动到 Git 分类的开头，目录中的「Git」变为粗体

#### Scenario: 窄编辑区隐藏目录

- **WHEN** 侧栏被拖宽，编辑区只剩 650 像素宽
- **THEN** 设置页面不显示目录，列表占满宽度

#### Scenario: 打开 JSON

- **WHEN** 用户点击标签栏右侧的「打开设置(JSON)」
- **THEN** 新增 `settings.json` 文件标签并成为当前标签，可编辑并用 ⌘S 保存

### Requirement: 设置项的显示

列表 SHALL 按分类分组，每组以分类名作为标题，组内按下表顺序显示设置项。每项自上而下 SHALL 依次是：

- **标题**：`<类别>: <名称>`，粗体，类别部分稍暗。
- **说明**：使用 VS Code 中文语言包的措辞。
- **控件**：
  - 枚举类型用 320 像素宽的下拉框，选项为取值本身，展开时显示每个选项的说明。
  - 布尔类型用复选框，放在说明文字左侧，点击说明文字也能切换。

| 分类 | 标题 | 说明 | 控件 |
|---|---|---|---|
| 工作台 > 外观 | Activity Bar: Location | 控制活动栏相对于主侧边栏的位置。 | 下拉框：`default`（在主侧边栏的侧边显示活动栏。）、`top`（在主侧边栏的顶部显示活动栏。）、`bottom`（在主侧边栏的底部显示活动栏。）、`hidden`（隐藏活动栏。） |
| 扩展 > Git | Git: Confirm Sync | 同步 Git 存储库前请先进行确认。 | 复选框 |
| 扩展 > Git | Git: Enable Smart Commit | 在没有暂存的更改时提交所有更改。 | 复选框 |
| 扩展 > Git | Git: Suggest Smart Commit | 建议启用智能提交(在无暂存更改时提交所有更改)。 | 复选框 |

鼠标悬停的行 SHALL 使用 VS Code 的 `settings.rowHoverBackground` 背景；获得焦点的行使用 `settings.focusedRowBackground` 背景。

#### Scenario: 列表内容

- **WHEN** 用户打开设置页面，`settings.json` 为空
- **THEN** 列表依次显示「外观」组的 Activity Bar: Location（下拉框显示 `default`），以及「Git」组的三项（Confirm Sync 和 Suggest Smart Commit 已勾选，Enable Smart Commit 未勾选）

### Requirement: 修改与重置

用户通过控件修改设置时，系统 SHALL 立即按 `settings` 的「写入设置」写入并生效。`settings.json` 中存在某个键时（无论值是否等于默认值），该项 SHALL 显示已修改标记：标题左侧有一条 2 像素宽的竖线，颜色为 VS Code 的 `settings.modifiedItemIndicator`。

鼠标悬停或焦点在某项上时，该项左侧 SHALL 显示齿轮按钮（「更多操作...」）。点击后弹出菜单：

- 「重置此设置」：删除该键，恢复默认值。只有该项已修改时才显示。
- 「复制设置 id」：把设置 ID（如 `git.confirmSync`）复制到剪贴板。
- 「将设置复制为 JSON 文本」：把 `"git.confirmSync": true` 形式的文本复制到剪贴板，值为当前生效的值。

#### Scenario: 修改后出现标记

- **WHEN** 用户在 Activity Bar: Location 下拉框中选择 `top`
- **THEN** 活动栏立即移到侧栏顶部，该项标题左侧出现竖线

#### Scenario: 重置

- **WHEN** Activity Bar: Location 当前为 `top`，用户点击它的齿轮按钮并选择「重置此设置」
- **THEN** 下拉框回到 `default`，竖线消失，活动栏回到侧栏左侧，`settings.json` 中不再有该键

#### Scenario: 复制设置 ID

- **WHEN** 用户点击 Git: Confirm Sync 的齿轮按钮并选择「复制设置 id」
- **THEN** 剪贴板内容为 `git.confirmSync`

### Requirement: 搜索设置

在搜索框中输入时，列表 SHALL 立即只显示匹配的设置项，并隐藏没有匹配项的分类标题。匹配规则：把输入按空格拆成若干词，每个词都要以不区分大小写的前缀方式，匹配到设置 ID、标题或说明中的某个词。

- 搜索框中含 `@modified` 时，SHALL 只显示已修改的项；它可以和其它词组合使用。
- 筛选按钮弹出的菜单中有「已更改」，点击后在搜索框中加入或去掉 `@modified`。
- 结果计数：有搜索内容时显示「找到 N 个设置」（只有 1 个时为「找到 1 个设置」）；没有匹配时，列表和目录都隐藏，只显示「未找到设置」；没有搜索内容时不显示计数。
- 点击清除按钮，或在搜索框中按 Esc，SHALL 清空搜索并恢复完整列表。
- 焦点在设置页面内时按 ⌘F，焦点 SHALL 回到搜索框并选中其中的文字。

#### Scenario: 按词搜索

- **WHEN** 用户在搜索框输入 `smart`
- **THEN** 列表只显示 Git: Enable Smart Commit 和 Git: Suggest Smart Commit，计数显示「找到 2 个设置」

#### Scenario: 只看已修改

- **WHEN** `settings.json` 中只有 `"git.confirmSync": false`，用户点击筛选按钮并选择「已更改」
- **THEN** 搜索框内容变为 `@modified`，列表只显示 Git: Confirm Sync，计数显示「找到 1 个设置」

#### Scenario: 没有结果

- **WHEN** 用户输入 `font`
- **THEN** 目录和列表隐藏，页面显示「未找到设置」

### Requirement: 与设置文件保持一致

设置页面显示的值和已修改标记 SHALL 始终与 `settings` 中当前生效的设置一致。`settings.json` 被外部修改，或通过询问框写入时，页面 SHALL 随之更新，并保持当前的搜索内容和滚动位置。

#### Scenario: 外部修改后页面更新

- **WHEN** 设置页面打开着，用户在 `settings.json` 标签中加入 `"git.enableSmartCommit": true` 并保存
- **THEN** 切回设置页面时，Git: Enable Smart Commit 已勾选并带已修改标记
