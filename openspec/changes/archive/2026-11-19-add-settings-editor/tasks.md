## 1. 准备

- [x] 1.1 修改 `openspec/config.yaml` 的项目 context，在功能边界中加入：VS Code 式设置页面（只列出 Whit 生效的设置，用户设置）和活动栏位置。并写明工作区设置下一期再做。验证：`openspec validate add-settings-editor --strict` 通过
- [x] 1.2 在 `Cargo.toml` 中加入 `jsonc-parser = { version = "0.34.0", features = ["cst", "serde"] }`，附注释说明用途，以及不用 Zed `settings_json` 的原因（GPL）。验证：`cargo check` 通过，`Cargo.lock` 中版本为 0.34.0

## 2. 编辑区提到窗口级（纯结构挪动，design 第 6 条）

- [x] 2.1 把 `TabKey::File` 改为绝对路径 `Arc<Path>`（canonicalize 后的路径），`FileBuffer` 改为只认绝对路径，显示名由调用方计算。仓库内的路径显示为相对路径，仓库外的显示为 `~` 开头。验证：`editor_group.rs` 和 `file_buffer` 的单元测试通过；新增单元测试，覆盖 `display_path` 的仓库内、仓库外、`~` 缩写三种情况
- [x] 2.2 把 `editors`、`buffers`、`hovered_tab`、`tab_scroll` 从 `Repository` 挪到 `Workbench`。`Repository::files_changed` 改为先用 `root.join` 拼出绝对路径再查 `buffers`；资源管理器定位只对仓库内的路径生效。验证：现有的 `tabs`、`unsaved`、`watching`、`explorer`、`file_buffer` 测试全部通过，行为不变
- [x] 2.3 关闭文件夹和切换仓库时，只关闭 diff 标签和仓库内的文件标签；`dirty_files` 增加按根目录过滤的参数，退出应用时询问全部。验证：新增测试：仓库外的文件标签在切换仓库后保留，且它有未保存修改时不参与询问
- [x] 2.4 没有仓库时，编辑区占满窗口；没有标签时，有仓库显示提示文字，无仓库显示欢迎页内容（`render_welcome`）。没有仓库且没有标签时，不画标签栏。验证：新增测试：无仓库时窗口只有欢迎页内容；关闭文件夹后，如果还有剩余标签，就显示标签栏且没有活动栏
- [x] 2.5 检查点：`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过；`tests/visual.rs` 中现有的截图场景全部通过；应用能启动，欢迎页的外观与改动前相同

## 3. SettingsStore：读取（design 第 1、2、5 条）

- [x] 3.1 在 `src/settings.rs` 中定义设置项表 `SETTINGS`（4 项，按 design 第 5 条的字段）、`ActivityBarLocation` 枚举，并把 `Values` 改为由表逐项填充。验证：单元测试确认表中每项的默认值与 VS Code 1.117 一致（`default`、`false`、`true`、`true`）
- [x] 3.2 实现 `SettingsStore`（`gpui::Global`）的 `load_from(file)` 和 `reload(text)`：JSONC 解析、按键容错、`configured` 集合、`last_text` 去重、`error`。验证：单元测试覆盖以下情况：
  - 带注释和尾逗号的文件能读出值；
  - 某个键的值类型不对时只影响该项，但它仍计入 `configured`；
  - 不认识的键被忽略；
  - 语法错误时保留上一次的值，并设置 `error`；
  - 文本相同时 `reload` 不触发更新；
  - 文件不存在时取默认值，不创建文件。
- [x] 3.3 在 `main.rs` 中安装全局 store；`Workbench::new` 去掉 `settings` 参数；`workbench.rs` 和 `scm_panel.rs` 中的读取改为读全局；`Workbench` 订阅 store 的变化并重绘。测试辅助函数改为在临时目录里 `set_global`。验证：现有的提交、同步询问测试全部通过，测试不读写用户真实的 `settings.json`

## 4. SettingsStore：写入与监视（design 第 3、4、9 条）

- [x] 4.1 实现 `SettingsStore::update(key, value)`：读出磁盘当前文本，在 jsonc-parser 的 CST 上最小编辑（等于默认值时删键，已有的键改值，没有的键追加），新建文件时用 4 个空格缩进，写临时文件再改名，写完立即 `reload`。语法错误时返回 `WriteError::Invalid`。验证：单元测试覆盖以下情况：
  - 注释和其它键原样保留；
  - 改回默认值时删除键；
  - 2 空格缩进的旧文件追加时仍用 2 空格；
  - 单行 `{ "a": 1 }`、尾逗号、只有注释的文件、空文件都能正确处理；
  - 删掉最后一个键后结果仍是合法的 JSONC；
  - 有语法错误时文件内容不变。
- [x] 4.2 把提交、同步询问框中的「始终 / 从不 / 不再显示」改为调用 `Workbench::write_setting`：先检查 `settings.json` 在编辑器里有没有未保存的修改，再调用 store；写入失败时弹出 VS Code 措辞的通知（「保存并重试」「打开设置」），提交或同步本身照常进行。验证：新增测试：`settings.json` 在编辑器中有未保存修改时选择「不再显示」，文件不变，弹出通知，同步照常执行；点击「保存并重试」后写入成功
- [x] 4.3 在 `watcher.rs` 中新增 `DirWatcher`：非递归监视若干目录，可以动态 `watch`/`unwatch`，复用 `gather` 的 100 ms 聚合，批次中是绝对路径。验证：真实 FSEvents 测试（与 `a_write_arrives_as_a_batch` 同样的写法）覆盖以下情况：
  - 改名替换文件能收到事件；
  - 同目录其它文件的事件也送达，由调用方按文件名过滤；
  - `unwatch` 后收不到事件。
- [x] 4.4 `SettingsStore` 用 `DirWatcher` 监视设置目录（启动时创建目录），收到 `settings.json` 或 `rescan` 时在后台读文件，再在主线程 `reload`；`cfg(test)` 下不启动监视器。`Workbench` 在 store 的 `error` 出现时弹出语法错误通知（带「打开设置」按钮），错误消失时关闭通知。验证：测试中调用 `reload` 写入坏文本，确认弹出通知且设置值不变；再 `reload` 好的文本，确认通知关闭、新值生效
- [x] 4.5 检查点：`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过；手动在终端修改真实的 `settings.json`，运行中的应用在 1 秒内生效（用 `tests/visual.rs` 的屏幕外窗口验证，不操作真实窗口）

## 5. 活动栏四种位置与视图切换（design 第 8 条）

- [x] 5.1 把资源管理器和源代码管理的标题行抽成公共的 `side_bar_title(label, actions)`。`top` 时用横排图标（35 像素高的格子，16 像素图标，底部 1 像素指示线）代替标题文字；`bottom` 时在侧栏底部加一行图标；`hidden` 时不画活动栏。活动栏变成横排时，徽标改用 compact 样式。验证：新增测试，对每种位置检查 `activity-explorer`、`activity-scm` 元素是否存在，以及「SOURCE CONTROL」文字是否显示
- [x] 5.2 `default` 位置时，在活动栏底部加「管理」齿轮（`activity-manage`），点击后弹出的菜单中有「设置 ⌘,」。图标的悬停提示带快捷键。验证：测试点击齿轮后菜单出现，点击「设置」后打开设置标签
- [x] 5.3 新增 `ShowExplorer`（⇧⌘E）、`ShowSourceControl`（⌃⇧G）动作和「查看」菜单：切换侧栏视图并聚焦（文件树，或提交信息输入框）；没有仓库时不起作用。验证：测试在 `hidden` 位置下按 ⇧⌘E，侧栏切换到资源管理器，且文件树获得焦点
- [x] 5.4 检查点：`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过；在 `tests/visual.rs` 中新增 `top`、`bottom`、`hidden` 三个截图场景，`top` 的截图与用户提供的 VS Code 顶部活动栏截图对照，图标间距、指示线、标题行按钮的位置一致

## 6. 设置页面（design 第 10 条，specs `settings-editor`）

- [x] 6.1 新增 `TabKey::Settings` 和 `src/workbench/settings_editor.rs` 中的 `SettingsEditor` 视图；新增 `OpenSettings`（⌘,，菜单「Whit → 首选项 → 设置」）动作，打开时插在当前标签右侧，已打开时切换过去，焦点进入搜索框。标签标题「设置」，带设置图标，不显示面包屑。验证：新增测试：有仓库和无仓库时按 ⌘, 都能打开；再按一次标签数量不变；无仓库时关闭设置标签后回到欢迎页
- [x] 6.2 页面结构：标题区（搜索框、计数、清除按钮、筛选按钮）、「用户」页签行、目录（200 像素）、列表。编辑区宽度小于 700 像素时隐藏目录。点击目录项滚动到对应分类，滚动列表时目录高亮当前分类。验证：测试点击「Git」后列表滚动到 Git 分组，目录中「Git」为选中状态；宽度 650 像素时找不到目录元素
- [x] 6.3 设置项的渲染：标题（类别稍暗、名称粗体）、中文说明，枚举用 320 像素宽的 `Select`（展开时显示每个选项的说明），布尔用 `Checkbox`（点击说明文字也能切换）；行悬停和焦点背景；已修改的项显示 2 像素竖线。控件的改动通过 `write_setting` 写入。验证：测试勾选 Git: Enable Smart Commit 后，`settings.json` 中有该键且出现竖线；把下拉框选回 `default` 后键被删除、竖线消失
- [x] 6.4 每项的齿轮菜单：「重置此设置」（只在已修改时显示）、「复制设置 id」、「将设置复制为 JSON 文本」。验证：测试复制后读剪贴板，内容为 `git.confirmSync` 和 `"git.confirmSync": true`；重置后键被删除
- [x] 6.5 搜索：按前缀匹配词、`@modified`，筛选菜单「已更改」，计数文案（「找到 N 个设置」「找到 1 个设置」「未找到设置」），Esc 和清除按钮清空搜索，⌘F 聚焦搜索框。验证：测试分别输入 `smart`、`@modified`、`font`，检查可见的设置项和计数文字
- [x] 6.6 「打开设置(JSON)」按钮（设置标签为当前标签时显示在标签栏右侧）和 `OpenUserSettingsJson` 动作：打开 `settings.json` 文件标签；文件不存在时照 VS Code 先以 `{` 换行 `}` 创建再打开。验证：测试点击按钮后出现 `settings.json` 标签，悬停提示为 `~` 开头的路径（在用户目录下时）；文件不存在时，点击后文件被创建、内容为 `{` 换行 `}`，设置仍是默认值
- [x] 6.7 检查点：`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过；在 `tests/visual.rs` 中新增设置页面截图场景（完整宽度、窄宽度、搜索 `@modified`、无仓库时打开设置），与本机 VS Code 1.117 的设置页面对照，布局和间距一致

## 7. 仓库外文件的监视与联动（design 第 7、9 条）

- [x] 7.1 `Workbench` 持有一个 `DirWatcher`：打开仓库外的文件时监视它所在的目录，该目录最后一个这样的标签关闭时停止监视；收到批次后，对命中的 buffer 调用 `disk_changed`。`cfg(test)` 下不启动，测试直接喂批次。验证：测试覆盖以下情况：
  - 没有未保存修改的 `settings.json` 标签被外部修改后静默重载；
  - 有未保存修改时保存，弹出冲突询问；
  - 文件被删除后，标签标题显示删除线。
- [x] 7.2 端到端联动：在 `settings.json` 标签中修改并保存后，store 生效（生产环境由监视器触发；测试中照监视器的做法调用 `reload` 模拟，保存路径本身不对 `settings.json` 做特殊处理），活动栏位置和设置页面随之更新；store 被询问框写入后，设置页面同步更新，搜索内容和滚动位置不变。验证：对应的两个测试通过
- [x] 7.3 总检查点：`cargo test`、`cargo clippy --all-targets -- -D warnings`、`openspec validate add-settings-editor --strict` 都通过；`tests/visual.rs` 中的全部截图场景通过；在 `docs` 或 `README` 中说明设置文件的位置以及支持的设置项
