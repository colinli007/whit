## Purpose

按 VS Code 用户设置的方式保存和读取 Whit 的设置：一个带注释也能读的 `settings.json`，键名与 VS Code 相同，界面和外部编辑都只改动到的键，改动立即生效。

## ADDED Requirements

### Requirement: 设置文件

用户设置 SHALL 保存在 `~/Library/Application Support/Whit/settings.json`，内容是一个顶层 JSON 对象，键名使用 VS Code 的设置 ID（如 `git.confirmSync`）。读取时 SHALL 允许 `//` 和 `/* */` 注释以及尾逗号（VS Code 的 JSONC）。以下情况下，系统 SHALL 按设置项的默认值工作，且不修改该文件：

- 文件不存在；
- 文件中缺少某个键；
- 某个键的值类型不对，或不在该设置允许的取值里（此时只影响这一项）。

Whit 不认识的键 SHALL 被忽略，并在写入时原样保留。

#### Scenario: 带注释的设置文件

- **WHEN** `settings.json` 内容为 `{ // 同步前不确认` 换行 `"git.confirmSync": false, }`
- **THEN** 同步时不弹出确认框

#### Scenario: 值不合法

- **WHEN** `settings.json` 内容为 `{ "git.confirmSync": "no", "workbench.activityBar.location": "top" }`
- **THEN** 同步时照默认值弹出确认框，活动栏显示在侧栏顶部

#### Scenario: 没有设置文件

- **WHEN** `settings.json` 不存在，用户启动 Whit
- **THEN** 所有设置取默认值，不创建该文件

### Requirement: 设置项

系统 SHALL 只提供以下设置，默认值与 VS Code 1.117 相同：

| 设置 ID | 类型 | 默认值 | 作用 |
|---|---|---|---|
| `workbench.activityBar.location` | `default` / `top` / `bottom` / `hidden` | `default` | 活动栏的位置，见 `explorer` 的「活动栏与侧栏视图」 |
| `git.enableSmartCommit` | 布尔 | `false` | 没有暂存的更改时，直接提交所有更改 |
| `git.suggestSmartCommit` | 布尔 | `true` | 没有暂存的更改时，询问是否提交所有更改 |
| `git.confirmSync` | 布尔 | `true` | 同步前确认 |

#### Scenario: 默认的活动栏位置

- **WHEN** `settings.json` 里没有 `workbench.activityBar.location`
- **THEN** 活动栏在侧栏左侧竖排显示

### Requirement: 写入设置

通过设置页面或询问框中的「始终 / 从不 / 不再显示」修改设置时，系统 SHALL 只修改 `settings.json` 中这一个键的值文本；文件中的其它键、注释、空行和缩进 SHALL 原样保留。新值等于默认值时，系统 SHALL 删除这个键，而不是写入默认值。文件中没有这个键时，SHALL 追加在对象末尾。文件不存在时，SHALL 新建，用 4 个空格缩进。写入 SHALL 先写临时文件再改名替换，写到一半中断不会留下损坏的文件。修改 SHALL 立即生效，没有「应用」步骤。

#### Scenario: 保留注释和其它键

- **WHEN** `settings.json` 内容为 `{ // 我的设置` 换行 `"editor.fontSize": 14 }`，用户在设置页面取消勾选 Git: Confirm Sync
- **THEN** 文件变为在 `"editor.fontSize": 14` 之后追加 `"git.confirmSync": false`，注释 `// 我的设置` 仍在

#### Scenario: 改回默认值删除键

- **WHEN** `settings.json` 中有 `"workbench.activityBar.location": "top"`，用户在设置页面选择 `default`
- **THEN** 文件中不再有 `workbench.activityBar.location` 这个键，活动栏回到侧栏左侧

#### Scenario: 询问框的选择也写入同一文件

- **WHEN** 用户在同步确认框中选择「确定，不再显示」
- **THEN** `settings.json` 中出现 `"git.confirmSync": false`，设置页面中该项的复选框取消勾选并显示已修改标记

### Requirement: 外部修改立即生效

系统 SHALL 监视 `settings.json`。在其它编辑器中修改、保存，或在 Whit 的编辑器中保存后，新的设置 SHALL 在 1 秒内生效，界面随之更新，不需要重启。如果文件被删除，SHALL 按默认值工作。

#### Scenario: 外部改了活动栏位置

- **WHEN** Whit 已打开仓库，用户在终端执行命令，把 `settings.json` 改为 `{ "workbench.activityBar.location": "top" }`
- **THEN** 1 秒内活动栏变为横排显示在侧栏顶部

#### Scenario: 在 Whit 中编辑 settings.json 后保存

- **WHEN** 用户在 Whit 的编辑器中打开 `settings.json`，加入 `"git.confirmSync": false` 并按 ⌘S
- **THEN** 之后同步时不再弹出确认框；设置页面中该项显示为未勾选且带已修改标记

### Requirement: 设置文件有语法错误

`settings.json` 存在但无法解析时（JSONC 允许的注释和尾逗号不算错误），系统 SHALL 继续使用上一次成功读取的设置；如果启动时就无法解析，则使用默认值。同时 SHALL 弹出一条不自动消失的错误通知「无法读取用户设置：settings.json 有语法错误。在修正之前，继续使用上一次读到的设置。」，附带「打开设置」按钮，点击后在编辑器中打开 `settings.json`。修正并保存后，新的设置 SHALL 生效，错误通知 SHALL 消失。

#### Scenario: 写坏后继续使用原来的设置

- **WHEN** `settings.json` 中有 `"workbench.activityBar.location": "top"`，用户在外部编辑器里删掉一个右花括号并保存
- **THEN** 活动栏仍在顶部，窗口弹出语法错误通知；点击「打开设置」后，编辑器打开 `settings.json`

### Requirement: 拒绝写入

在以下两种情况下，通过设置页面或询问框修改设置时，系统 SHALL 不写入文件、不改变当前设置，并弹出错误通知（文案与 VS Code 相同）：

- `settings.json` 在 Whit 的编辑器中有未保存的修改：通知「由于该文件具有未保存的更改，因此无法写入到用户设置。请先保存该用户设置文件，然后重试。」，按钮「保存并重试」（保存该文件，再重新执行这次修改）和「打开设置」。
- `settings.json` 有语法错误：通知「无法写入用户设置。请打开用户设置并清除错误或警告，然后重试。」，按钮「打开设置」。

#### Scenario: 编辑器里有未保存的修改

- **WHEN** `settings.json` 在编辑器中有未保存的修改，用户在设置页面勾选 Git: Enable Smart Commit
- **THEN** 弹出「由于该文件具有未保存的更改…」通知，复选框回到未勾选；点击「保存并重试」后，编辑器中的修改被保存，`git.enableSmartCommit` 被写为 `true`

#### Scenario: 文件有语法错误时修改

- **WHEN** `settings.json` 有语法错误，用户在设置页面把活动栏位置改为 `top`
- **THEN** 弹出「无法写入用户设置…」通知，活动栏位置不变，文件内容不变
