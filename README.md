# Whit

git 改动查看与提交工具，布局和交互参照 VS Code 的源代码管理面板（VS Code 1.117）。界面和代码配色是 VS Code 的 Monokai Dimmed，代码字体 Fira Code 14px（带连字；本机没装时退回系统等宽字体），文件图标是 vscode-icons（见下面「文件图标」）：

- **欢迎页**：未打开文件夹时显示，「最近」列出最近打开的仓库（最多保存 50 个，默认显示 10 个）。
- **源代码管理侧栏**：提交信息（⌘Enter 提交）；输入框下方的操作按钮和 VS Code 一样随仓库状态变化：有改动时是带下拉菜单的「提交」（提交 / 提交(修改) / 提交并推送 / 提交并同步），没有改动时是「同步更改 N↓ M↑」或「发布分支」，合并或变基停在冲突上时是「继续」；「合并更改」「暂存的更改」「更改」三组文件，文件行悬停时可打开、暂存、取消暂存、放弃更改。
- **编辑区**：点击文件显示这个文件完整的左右对照 diff（不折叠未改动的行）；长行在每栏内裁切、两栏同步横向滚动；可切换并排 / 内联，跳到上一处 / 下一处更改。和 VS Code 一样画缩进参考线和括号配对参考线（在代码里点一下，当前缩进块和当前括号对会高亮），最右侧是改动标记条（左道删除、右道新增，灰色滑块是当前可见范围，可点击、拖动、滚动）。
- **状态栏**：分支名（有改动时带 `*`），点击打开「签出到」；`↓ ↑` 待拉取 / 待推送提交数，点击同步；分支没有上游时显示「发布分支」；分离头指针时不显示同步入口。
- **「…」菜单**（VS Code 的结构）：拉取、推送、签出到...、抓取，以及「提交」「更改」「拉取，推送」「分支」四个子菜单。
- **分支**：VS Code 式的快速选择框（窗口顶部居中，可输入过滤）。签出分支 / 远程分支 / 标记（远程分支自动建立本地跟踪分支），签出被本地改动挡住时可「储藏并签出 / 迁移更改 / 强制签出」；创建分支、从指定起点创建、签出已分离；重命名、删除本地 / 远程分支；合并、变基，冲突解决并暂存后点「继续」，也可中止合并 / 变基；发布分支（多个远程时选择推到哪个）。
- **⌘J 在终端中打开**：用 Ghostty 新开一个窗口或标签，工作目录是当前仓库根目录（也在「文件」菜单里）；需要本机装有 Ghostty。

窗口重新获得焦点、按 cmd-R、以及每次写操作之后都会自动刷新。

## 写操作的注意事项

- 暂存、提交、拉取、推送都直接调用本机 `git`，遵循你自己的 git 配置，会执行仓库的钩子（pre-commit 等）和提交签名，和在终端里执行一样。
- 打开、浏览仓库不会执行仓库配置里的外部程序（读取时关闭了 fsmonitor、外部 diff 和 textconv）。
- 远程认证只用本机已配置的凭据（ssh-agent、钥匙串、凭据助手）。需要输入密码或口令时操作会直接失败并提示，请在终端里配置好认证后重试。
- 放弃未跟踪文件会永久删除它，执行前会二次确认。
- 签出时选「强制签出」会丢掉全部未提交的改动并删除未跟踪文件；删除未完全合并的分支前会再确认一次。
- 最近项目列表保存在 `~/Library/Application Support/Whit/recent.json`。

## 下载安装

在 [Releases](https://github.com/colinli007/whit/releases/latest) 下载 `.dmg`：Apple 芯片选 `aarch64`，Intel 选 `x64`。需要 macOS 12 及以上，本机装有 `git`。

打开 `.dmg`，把 Whit 拖进「应用程序」。安装包没有经过 Apple 公证，第一次打开会提示「无法验证开发者」，二选一：

- 在「系统设置 → 隐私与安全性」底部点「仍要打开」；
- 或在终端执行 `xattr -dr com.apple.quarantine /Applications/Whit.app`，之后正常双击打开。

发布新版本：改 `Cargo.toml` 的 `version` 并提交，然后推送同名标签（如 `git tag v0.2.0 && git push origin v0.2.0`），GitHub Actions 会打出两种架构的 `.dmg` 并附到这个标签的 Release 上。

## 构建前提

- Rust（stable，edition 2024）
- macOS 需要 Xcode 的 Metal Toolchain（Xcode → Settings → Components），gpui 编译着色器时要用
- 运行时需要本机安装 `git`

## 运行

```sh
cargo run --release -- <仓库路径>
```

不带路径启动时显示欢迎页，用「打开…」、cmd-O 或最近项目打开文件夹。

## 测试

- `cargo test`：git 读写的单元测试，以及用 gpui 测试平台驱动整个窗口的无头界面测试（点击、按键、确认框都是模拟的，不显示窗口）。
- `cargo test --features visual-tests --test visual`：截图测试。用真实的 macOS 平台和 Metal 渲染，窗口开在屏幕外（-10000, -10000）、不获取焦点，事件直接注入应用，截图从渲染结果读出，存到 `target/visual-shots/`。不会移动鼠标、不抢焦点，锁屏时也能跑。原生确认框在这里换成窗口内绘制的版本（按钮 id 为 `prompt-<文字>`），以便截图和点击。AppKit 必须在主线程上运行，所以这个测试目标自带 `main`（`harness = false`），场景在 `tests/visual.rs` 的 `main` 里登记。

## 文件图标

侧栏文件行、编辑器标签和面包屑的文件图标来自 [vscode-icons](https://github.com/vscode-icons/vscode-icons)，放在 `assets/file-icons/`，编译时嵌入程序。选图标的规则和 VS Code 一样：文件名 → 最长的扩展名 → 文件所属语言 → 默认图标。

`assets/file-icons/` 由 `python3 scripts/file_icons.py` 生成，它读取 `/Applications` 里的 VS Code 和已安装的 vscode-icons 扩展：扩展的图标主题，以及 VS Code 内置扩展声明的语言（哪些扩展名、文件名属于哪种语言）。不读其他扩展和用户设置，所以同样版本的 VS Code 和 vscode-icons 在任何机器上都生成同样的结果。两者有更新时重新跑一次即可。图标按 CC BY-SA 4.0 授权，扩展代码按 MIT 授权（见 `assets/file-icons/README.md`），品牌图标的版权归各自所有者。

## 内置的 gpui-component 补丁

`vendor/gpui-component` 是 gpui-component 0.9.1 的副本，通过 `Cargo.toml` 的 `[patch.crates-io]` 替换上游版本，许可证是 Apache 2.0（`vendor/gpui-component/LICENSE-APACHE`）。改动有七处：

- diff 组件不换行时，左右两栏各占一半宽度、在栏内裁切代码，所有行和两栏共用一个横向偏移同步滚动（和 VS Code 的并排 diff 一样）；上游是让每栏宽度等于最长一行，长行会把右栏挤出窗口。
- 新增 `Diff::change_colors`，可以直接指定增删行和增删词的底色（用来对齐 VS Code 的 `diffEditor.*` 颜色）；上游只能用主题色乘固定透明度。
- 语法高亮不再输出 locals 查询的 `local.*` 捕获：上游只禁用了 injections 段，locals 段的捕获会盖掉 `variable.parameter` 等真正的高亮名，导致 JS/TS/TSX/Ruby/Scala/Lua/Svelte 的参数等不上色。

- 命令面板 `Command` 新增 `search_icon(bool)`（默认开），快速选择框用它去掉搜索框前的放大镜，和 VS Code 一致。
- TSX 用完整的 TypeScript 高亮查询加 JSX 规则，JavaScript 也补上 JSX 规则：上游 TSX 直接用 tree-sitter-typescript 的 `HIGHLIGHTS_QUERY`，它只是 TypeScript 在 JavaScript 之上新增的那几十行（本该和 JS 规则叠加使用），结果 `.tsx` 文件里关键字、字符串、函数名、数字、JSX 标签都不上色。
- 让 diff 看起来和 VS Code 一样：
  - `Diff::line_height_ratio`（行高 = 字号 × 比例，取整）；代码字号直接用主题的 `mono_font.size`（和组件的代码编辑器一致），上游会再乘 rem/16。行号用主题的 `editor.line_number` 颜色。
  - `Diff::guides(DiffGuideColors)`：按 VS Code 的算法画缩进参考线和括号配对参考线（`src/diff/guides.rs`）。括号对来自语法树，所以字符串、注释里的括号不算；「光标」是鼠标最后在代码里按下的位置。
  - `Diff::overview_ruler(true)`：VS Code 的 diff 改动标记条（`src/diff/overview.rs`）。
  - 主题的 `highlight.syntax` 可以写任意捕获名（如 `type.builtin`），找不到时逐级去掉后缀回退（`type.builtin` → `type`）；上游只认固定的 41 个名字，而且只回退到第一段。
  - JS/TS/TSX 高亮规则按 VS Code 的 TextMate 作用域细分：`new` / `typeof` 等是 `keyword.operator`，三元运算符、类型注解的冒号、`...` 是运算符，`number` 这类内置类型是 `type.builtin`，`new X()` 的 X 是 `constructor`，模板字符串 `${ }` 的两个括号都是 `punctuation.special`，JSX 组件标签是 `tag.component`、尖括号是 `tag.delimiter`、`{ }` 是 `punctuation.section.embedded`。
- 菜单、按钮提示和通知按 VS Code 1.117 的尺寸绘制：上游用的是以 rem 计的预设，而主题字号 13 让 rem 也是 13px，预设整体比 VS Code 小。菜单为 13px 文字、24px 行高、6px 圆角、左右 26px 内边距、上下 4px 留白、分隔线 1px 上下各 5px（颜色是前景色的 20%，即 VS Code 的 `menu.separatorBackground`）、最小宽度 160px、外框 8px 圆角；提示为 12px 文字、19px 行高、2px 8px 内边距、5px 圆角；挂在组件上的提示（按钮等）照 VS Code 带指针的悬停框：默认显示在下方、离触发元素 1px、3px 圆角，并画出指向触发元素的小箭头（被窗口边缘推开时箭头仍对准触发元素）；通知为 13px 文字、22px 行高、10px 5px 内边距、18px 图标、4px 圆角（位置和宽度由应用在 `NotificationSettings` 里设）。

前两处存成 `patches/gpui-component-split-scroll.patch`，第三处存成 `patches/gpui-component-highlight-locals.patch`，第四处存成 `patches/gpui-component-command-search-icon.patch`，第五处存成 `patches/gpui-component-jsx-highlights.patch`，第六处存成 `patches/gpui-component-vscode-editor.patch`（要在第五处之后打），第七处存成 `patches/gpui-component-vscode-popups.patch`。最后再打 `patches/gpui-component-modification-notices.patch`：按 Apache 2.0 的要求，在每个改过的文件开头注明已修改。升级 gpui-kit 时：把新版 gpui-component 复制到 `vendor/gpui-component`，在该目录依次执行 `patch -p1 < ../../patches/<补丁文件>`，冲突时手动合并，然后跑 `cargo test` 和组件自己的 diff 测试（`cargo test --lib diff::`；组件的单测引用了上游仓库里的图标和主题文件，单独跑时需要按上游仓库的目录结构摆放）。

## 许可证

MIT，见 `LICENSE`。内含或改编的第三方作品（gpui-component、VS Code、tree-sitter-javascript、vscode-icons）保留各自的许可证，见 `THIRD-PARTY-NOTICES.md`。
