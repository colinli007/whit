## 1. 准备

- [x] 1.1 修改 `openspec/config.yaml` 的项目 context：把「分支创建与切换」从不做的列表中移除，写明本期做分支全套，储藏 / 标记 / 远程管理放到下一期，克隆和工作树不做；验证：`openspec validate add-branch-management --strict` 通过
- [x] 1.2 `GitError` 新增 `DirtyWorkTree`、`BranchNotFullyMerged`、`Conflict`，`NoRemote` 的文案改为「仓库未配置任何要发布到的远程仓库。」；新增 `remotes()` 读取远程名与推送地址，`publish` 改为接受远程名，选远程的规则暂时搬到 `Workbench` 里（`AmbiguousRemote` 的删除随 5.7 的远程选择列表一起做，那时才有替代它的界面）；验证：`cargo check` 通过，现有测试全部通过

## 2. git 读取（`src/git/read.rs`）

- [x] 2.1 实现 `list_references`：一次 `for-each-ref --sort=-committerdate` 读出本地分支、远程分支、标记，包括提交作者、时间、标题（附注标记取所指向的提交）、上游和领先 / 落后数，跳过 `<远程>/HEAD`；验证：单元测试覆盖按时间排序、附注标记与轻量标记、有 / 无上游的领先落后、远程分支的远程名拆分、分支名带 `/`
- [x] 2.2 `Snapshot` 增加 `in_progress: Option<Merge | Rebase>`：每次刷新用 `rev-parse --git-path` 取得 `MERGE_HEAD`、`rebase-merge`、`rebase-apply` 的路径并检查这些文件，结果写进 `key`；再加一个读取 `MERGE_MSG`（去掉 `#` 注释行）的函数；验证：测试分别制造合并冲突、变基冲突，断言 `progress` 的值，中止后回到 None；在 worktree 中也能读对
- [x] 2.3 实现 `relative_time` 和 `sanitize_branch_name`（design 第 8、9 节）；验证：单元测试覆盖各时间阈值的边界（29 秒、30 秒、59 分钟、1 小时、6 天、7 天、30 天、365 天），以及空格、`..`、`~^:`、开头 `-`、结尾 `.lock`、结尾 `/` 的替换

## 3. git 写操作（`src/git/write.rs`）

- [x] 3.1 实现签出（分支 / 标记 / `--track` 远程分支 / `--detach`）和创建分支（`checkout -q -b <name> --no-track <start>`），stderr 匹配到「要先提交或储藏」时返回 `DirtyWorkTree`；验证：测试覆盖切换分支、从远程建立跟踪分支、分离签出、从指定起点创建分支、有冲突的改动返回 `DirtyWorkTree`、不冲突的改动保留
- [x] 3.2 实现储藏（`stash push --include-untracked`）、弹出（`stash pop`）、放弃全部（`reset --hard` + `clean -fd`）；验证：测试覆盖储藏后工作区干净（含未跟踪文件）、弹出后改动与未跟踪文件恢复、弹出冲突返回 `Conflict`、放弃全部后 status 为空
- [x] 3.3 实现重命名、删除（`-d`，stderr 匹配 not fully merged 时返回 `BranchNotFullyMerged`；`-D` 强制删除）和删除远程分支（`push <remote> --delete`，走 `remote_command`）；验证：用本地裸仓库作远程做测试，覆盖未合并分支的两步删除、删除远程分支后 `ls-remote` 中不再有该分支
- [x] 3.4 实现合并、变基、`rebase --continue`（`GIT_EDITOR=true`）、`merge --abort`、`rebase --abort`；失败后有未合并条目时返回 `Conflict`；验证：测试覆盖无冲突合并、合并冲突后返回 `Conflict` 且 `progress` 为 Merge、暂存后提交完成合并、变基冲突后继续完成、两种中止
- [x] 3.5 检查点：`cargo test` 与 `cargo clippy --all-targets -- -D warnings` 全部通过
  - `cargo test` 70 个全部通过；clippy 此时只剩「未使用」告警（这些函数的调用方在第 5 组），没有其它问题，`-D warnings` 的完整通过由 5.9 检查

## 4. 快速选择框（`src/workbench/quick_input.rs`）

- [x] 4.1 实现 `pick`：覆盖层定位（顶部居中、宽度 `min(窗口宽 × 0.62, 600)`），基于 `Command` + `filterable(false)` 自己过滤和排序，分组名画在该组第一行右侧，用 `StyledText` 高亮匹配的部分，两行条目，始终显示的条目，没有匹配时显示「没有匹配的结果」，返回 `oneshot::Receiver<Option<T>>`；验证：无头界面测试覆盖打开后焦点在输入框、输入过滤（不区分大小写、分组名随过滤消失）、有输入时操作条目排在分隔线后、回车返回所选项、Esc / 点外部 / 窗口失焦返回 None、打开第二个时第一个收到 None
- [x] 4.2 在 4.1 的基础上加入加载状态（先打开、后填条目，已输入的文字保留）和 ⌘↑ / ⌘↓ / PageUp / PageDown 键位；验证：无头测试中延迟填入条目时，加载前输入的文字在条目到达后生效；键位能跳到首项和末项
- [x] 4.3 实现 `input`：占位文字、提示行、校验结果三种样式、`Error` 时回车不生效、初始值及预选范围、点外部不关闭；验证：无头测试覆盖重名报错时回车无效、名字含空格时显示「新分支将为“…”」并在回车后返回输入的原文（规范化由调用方做，见 5.3）、Esc 返回 None、点外部仍保持打开
- [x] 4.4 检查点：`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过；等 5.2 接好状态栏入口后启动应用打开快速选择框（不写临时入口），截图核对位置、宽度、两行条目、组名位置与 VS Code 截图一致
  - `cargo test`、`cargo clippy --all-targets -- -D warnings` 通过；截图核对与 6.1 的真实窗口验收合并进行

## 5. 流程编排与操作按钮（`src/workbench/`）

- [x] 5.1 新增 `run_operation_with`（返回 `Task<Result<(), GitError>>`，占用同一个「执行中」位置，结束后刷新，不自动弹通知），`run_operation` 改为在它上面的薄包装；验证：现有测试全部通过
- [x] 5.2 实现「签出到」：状态栏分支名可点击（写操作进行中不可点），「…」新增「签出到...」；列表按 spec 组织（三个操作条目，「分支 / 远程分支 / 标记」三组的说明和详情）；本地 / 远程 / 标记三种签出；遇到 `DirtyWorkTree` 时弹确认框并执行「储藏并签出 / 迁移更改 / 强制签出」；验证：无头测试覆盖点状态栏打开列表、签出本地分支后状态栏和占位文字更新、签出远程分支建立跟踪、已有跟踪分支时签出本地分支、三种脏工作区选项及取消
- [x] 5.3 实现「创建新分支... / 创建新分支依据... / 签出已分离…」以及菜单中的「创建分支... / 从现有来源创建新的分支...」；验证：无头测试覆盖签出列表中已输入文字时直接创建、未输入时弹输入框、选择起点后创建、重名报错、分离签出后状态栏显示短哈希
- [x] 5.4 实现「重命名分支... / 删除分支... / 删除远程分支...」；验证：无头测试覆盖重命名后状态栏更新、分离头指针时重命名不可用、删除未合并分支时的二次确认（确认与取消）、删除远程分支后签出列表中不再出现
- [x] 5.5 实现「合并... / 变基分支…」，以及冲突时的警告通知「存在合并冲突。请在提交之前解决这些冲突。」；变基列表的上游排第一并标注「(upstream)」；验证：无头测试覆盖无冲突合并、冲突合并后文件出现在「合并更改」组并出现警告、变基列表第一项是上游
- [x] 5.6 合并 / 变基进行中：`ActionButton` 增加 `Continue`（存在未解决冲突时不可用），合并时预填 `MERGE_MSG`（只在状态从 None 变为 Merge 时填一次），变基时继续执行 `rebase --continue`；「提交」子菜单在变基进行中显示「中止变基」，「分支」子菜单在合并进行中显示「中止合并」；验证：无头测试覆盖解决冲突后点「继续」完成合并、变基冲突解决后继续完成、两种中止后按钮恢复
- [x] 5.7 发布分支：删除 `GitError::AmbiguousRemote`；`ActionButton` 增加 `Publish`（没有改动、处于某个分支、没有上游时显示，次要样式，云上传图标）；有多个远程时打开远程选择列表，没有远程时提示；验证：无头测试覆盖新建分支后按钮显示「发布分支」并能发布、两个远程时出现选择列表并按所选远程设置上游、没有远程时出现提示通知
- [x] 5.8 「…」菜单按 spec 改为顶部四项加「提交 / 更改 / 拉取，推送 / 分支」四个子菜单，「获取」改名为「抓取」，菜单项的可用状态按 spec 处理；验证：无头测试打开「…」菜单和各个子菜单，断言条目顺序；写操作进行中时条目不可用（按以往做法：点击后没有效果，再用正常情况作对照）
- [x] 5.9 检查点：`cargo test` 与 `cargo clippy --all-targets -- -D warnings` 全部通过

## 6. 验收与收尾

- [x] 6.1 真实窗口验收（按项目记忆中的界面验证方法：先确认 diffview 在最前再点击）：在一个有多个分支、远程和标记的测试仓库中，依次走完点状态栏签出、迁移更改、创建分支后发布、合并冲突后继续、变基后中止、删除分支，并截图与 VS Code 截图对比快速选择框的样式；验证：每一步截图保存在 scratchpad，结果写进任务备注
  - 2026-11-16 在 scratchpad 的演示仓库（5 个分支、远程、两个标记）上用调试版程序逐步操作，截图在 scratchpad/tools/01–20：点状态栏打开签出列表（样式与 VS Code 截图一致：宽 600、两行条目、组名在右、`main-next 1↓ 0↑`）；签出 main-next；本地改动挡住签出时弹出「储藏并签出 / 迁移更改 / 强制签出 / 取消」，迁移后到 dev 且改动保留；「…」菜单与「分支」子菜单结构正确（有上游时「发布分支...」置灰）；合并 main 冲突 → 警告通知、「合并更改」组、「继续」置灰、输入框预填 `Merge branch 'main' into dev`，命令行解决并暂存后点「继续」完成合并；「创建分支...」输入框样式正确；新分支显示「发布分支」按钮，点击后上游为 origin/feature/demo；删除分支列表不含当前分支；变基列表首项为 `origin/dev (upstream)`，变基冲突后「提交」子菜单出现「中止变基」，点击后回到原提交。
  - 与 VS Code 的差异：快速选择框输入框左侧有组件自带的放大镜图标；分离头指针（含变基进行中）时状态栏同步入口和「…」顶部那项仍显示「发布分支」（上一期的既有行为，VS Code 此时不显示发布）。两处已按用户确认在 6.3 修正。
- [x] 6.2 更新 README 的功能说明和项目记忆（范围边界、新增的快速选择框组件）；验证：README 中列出分支相关功能，`openspec validate add-branch-management --strict` 通过
- [x] 6.3 按验收发现的两处差异修正（用户 2026-11-16 确认）：给 vendor 的 gpui-component `Command` 加 `search_icon` 选项（默认开），快速选择框关掉放大镜图标，改动存成 `patches/gpui-component-command-search-icon.patch`；分离头指针时状态栏不显示同步入口、「…」顶部那项显示「推送」，规格同步补充；验证：补丁在原文件副本上 `patch -p1` 后与 vendor 一致，`checking_out_detached_shows_the_commit` 增加对应断言，`cargo test` 101 个通过，`cargo clippy --all-targets -- -D warnings` 通过
