# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# Whit 项目规则

Whit 是仿 VS Code 1.117「源代码管理」的 macOS git 客户端（查看改动、暂存、提交、分支操作，外加资源管理器、文件编辑和设置页），Rust 单 crate，界面基于 GPUI（只依赖总包 `gpui-kit`）。功能边界和「不做的事」见 `openspec/config.yaml` 的 `context`。

## 对齐原则：界面看 VS Code，实现看 Zed

- **界面、交互、图标一律以 VS Code 为准**（本机安装的 VS Code 1.117）。尺寸、间距、颜色、行为都从 VS Code 源头查，不凭感觉近似：
  - 样式：`/Applications/Visual Studio Code.app/Contents/Resources/app/out/vs/workbench/workbench.desktop.main.css`；命令、菜单、图标注册：同目录的 `workbench.desktop.main.js`；内置扩展（如 git）：`extensions/<名字>/package.json`。
  - 图标用 VS Code 的 codicons：先查出 VS Code 在这个位置用的 codicon 名（如 `git.openFile` → `go-to-file`），把 `@vscode/codicons` 包里的 SVG 原样放进 `assets/codicons/icons`，再在 `src/codicons.rs` 的 `Codicon` 枚举里加一项。组件库内部按名字画的 Lucide 图标（菜单勾选、通知、diff 折叠条等），由 `codicons.rs` 的 `COMPONENT_ICONS` 在加载时换成对应的 codicon，组件新用到一个图标就在表里补一行。代码里不要再直接用 Lucide。action 栏图标用 `icon.foreground`（#C5C5C5），禁用时透明度 0.6，都已经收在 `scm_panel.rs` 的 `icon_button` 里。
  - 颜色取 Monokai Dimmed 主题的值；主题没设的，用 VS Code 暗色默认值（放在 `src/theme.rs` 的 `colors`）。
  - 界面文字用简体中文，与装了中文语言包的 VS Code 一致。
- **代码实现，尤其是性能相关的部分，参考 Zed**：渲染与重绘、虚拟列表、文件监听、git 调用、增量更新、后台任务等。参考时注明 Zed 的文件路径，只重写、不复制 GPL 许可的代码。
- 两者冲突时：用户看到的效果和行为按 VS Code，背后怎么实现按 Zed。

## 常用命令

```sh
cargo run --release -- <仓库路径>          # 不带路径启动显示欢迎页
cargo test                                  # git 读写单测 + 无头界面测试（gpui 测试平台，不弹窗口）
cargo test --lib workbench::tests::tabs     # 只跑某一组界面测试；单个测试再加 ::<测试名>
cargo test --features visual-tests --test visual   # 屏幕外截图测试，截图写到 target/visual-shots/
cargo clippy -- -D warnings                 # 每组任务的检查点之一
python3 scripts/file_icons.py               # VS Code 或 vscode-icons 升级后重新生成 assets/file-icons/
```

- 编译需要 Xcode 的 Metal Toolchain（gpui 编译着色器）。
- 截图测试用真实 MacPlatform + Metal 离屏渲染，窗口在 (-10000,-10000) 且不获取焦点，不抢鼠标、锁屏也能跑。它是 `harness = false` 的独立目标（AppKit 要主线程），场景在 `tests/visual.rs` 的 `main` 里注册，没有按名过滤，一次跑全部。界面验证用这两类测试，不要用 CGEvent 去点真实窗口。
- 发版：改 `Cargo.toml` 的 `version`，推同名 `v*` 标签，`.github/workflows/release.yml` 用 cargo-packager 打两种架构的 dmg。
- 提交信息一律英文（标题和正文）。

## 架构

- `src/main.rs` 只开窗口；其余都在库里（`src/lib.rs`：全部 action、快捷键、菜单、`AppAssets`、`init`），这样截图测试能驱动同一个窗口。新快捷键/菜单项都在 `lib.rs` 的 `init` 里注册，测试也调用它。
- `src/workbench.rs` 的 `Workbench` 是唯一的根视图，持有全部共享状态：`Repository`（打开的仓库：git 快照、diff、提交信息、正在运行的 `Operation`、刷新任务、文件监听）、`Editors`（编辑区标签，属于窗口而不是仓库）、Quick Input。各区域（`src/workbench/` 下的 `scm_panel`、`explorer_panel`、`editor`、`status_bar`、`activity_bar`、`welcome`、`settings_editor`）各自一个文件渲染，读写的都是这份状态。
- git：一律调用本机 `git` 命令（不用 gix/git2），所有命令都在 `src/git/mod.rs` 构造：只读命令带 `--no-optional-locks`、关 fsmonitor，不执行仓库配置里的外部程序；写命令（`write.rs`、`branches.rs`）按用户配置跑 hooks 和签名。`LC_ALL=C` 是因为错误分类读 git 的英文信息。`read.rs` 一次刷新产出一个 `Snapshot`（分支状态 + 按「合并更改 / 暂存的更改 / 更改」分组的文件及各自 patch）。
- 刷新：替换 `refresh_task` 即取消在途刷新，只有最新一次能更新视图；同一时间只跑一个 `Operation`。`src/watcher.rs` 用 notify（FSEvents）攒批次后交给窗口，照 Zed worktree 的做法。
- diff 展示用 gpui-component 的只读 `Diff` 组件（输入是 unified patch 文本）。所有 diff 标签共用一个 `DiffState`；patch 文本没变就不要重新 `set_files`（会重置滚动、折叠和选区）。`Diff` 必须始终挂载：gpui 在绘制阶段收到的 notify 会被吞，按需挂载会偶发整片空白。
- 分支等多步流程（`workbench/branches.rs`）写成一个 async 函数，按顺序 `await` Quick Input 的回答，像 VS Code 的 `await showQuickPick(...)`。
- 设置（`src/settings.rs`）：`~/Library/Application Support/Whit/settings.json` 是唯一记录，用 VS Code 的设置名，jsonc 读、按键增量写回并保留注释和格式；`SettingsStore` 是全局，视图观察它。

## vendor 的组件库

`vendor/gpui-component`、`vendor/gpui-base` 通过 `[patch.crates-io]` 替换上游。要改组件行为就改 vendor 里的代码，并**同步更新 `patches/` 里对应的补丁文件**；补丁顺序和升级 gpui-kit 的步骤见 README「Bundled gpui-component patches」。gpui-kit 版本被精确锁定（`=0.9.1`），它又精确锁定 `gpui-pre`，两者要一起升级。

## 测试里的坑

- 界面测试在 `src/workbench/tests/`，用 `#[gpui_kit::test]`；模块里需要 `use core::prelude::v1::test;`，否则 `gpui_kit::*` 带进来的 `test` 宏会导致递归展开。git 仓库夹具在 `src/git/mod.rs` 的 `fixtures`。
- 测试调度器是确定性的，任何其它 OS 线程（如真实文件监听线程）唤醒它都会 panic；单测里直接喂监听批次，真实监听放到截图测试里验证。
- `window.dispatch_action` 延迟到本次 update 结束才派发；`run_until_parked` 会等后台 git 进程结束，测「进行中」状态时不要 park。
- 截图测试里计时器走模拟时钟，等提示/通知动画用 `Visual::wait`。
- gpui-kit 的 rem 是 13px，组件预设尺寸都比 VS Code 小：按 VS Code 的 px 精确写，图标和文字包成一个子元素传给按钮（见 `scm_panel.rs` 的 `icon_button` / `text_button`）。`group_hover` 不能切换 display（会 panic），按悬停改布局用 `on_hover` 把状态记进视图。

## OpenSpec

功能按 OpenSpec 流程走（`openspec/specs/` 是现行规格，`openspec/changes/` 是提案），文档全部用简体中文；`openspec/config.yaml` 的 `rules` 规定了提案要有「不做的事」、设计要说明性能瓶颈和 Zed 出处、任务要有可验证检查点。
