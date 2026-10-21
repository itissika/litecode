# Bash 进入 Code `run_command` — 执行计划

分支：`feat/tool-contract-ptc`。只写计划，不改产品代码，不提交，不推送，不改版本号。

权威：`docs/ptc/charter.md` 的不变量，加上 Andy 2026-10-21 的三条产品目标。`AGENTS.md` 不改。`dev/archive/` 与 `status: pending` 的知识节点不是决策。

**章程范围句被本分支覆盖。** `docs/ptc/charter.md` D2 与 §4 仍写「bash 系列保持 Direct / 本轮不做 bash code 化」。本计划按 Andy 的三条目标做完 bash 能力，不改章程文件（只有编排者改章程）。仍然生效的不变量：I1 单调用面、I2 投影与执行双拒绝、I4 准入、I5 同一条管线、I7 取消是封口、I8 权限只收紧、I9 不新增 Item 种类、I12 成功 resolve data / 领域失败 reject `ToolError`。不要改 JS runtime 去保住未完成的 `await`。

## 0. 事实勘误（相对任务书，已对代码）

任务书里下列说法成立，照用：`run_command` 现为 Code、仅前台、tokio 管道、stdout/stderr 分开、超时杀进程组、无 job id；日志在 `.litecode/run_command/`，stem 是 `terminal::bash_nonce()` 的 8 hex，因为嵌套 `call_id` 是 `{parent}:code:{index}`（`src/tools/code.rs` 约 652 行），`:` 在 Windows 文件名上是 os error 123。`shell_command`、`bash_safety`、`INLINE_*` / `FILE_MAX` 已共享。权限 floor / preset / `authorize` 已把 `bash | run_command` 写在一起。default / general / orchestrator 经 `bind_configurable(..., All)` 绑 `run_command`；explore 在 `seed_explore_agent_bindings` 里对 `run_command` 用 SAFE。I7：程序结束先取消在途嵌套调用；后台作业只有在嵌套调用已经返回 job id、且进程归属在程序之外时才能活下来。

需要改口的事实：

1. **JS 线程不是 tokio current_thread。** `ptc::execute`（`src/ptc/mod.rs` 约 445–466 行）把程序放在名为 `ptc-js` 的 OS 线程上，用 `thread::park` 驱动（`src/ptc/engine.rs` 的 `block_on`）。嵌套宿主 future 由 `handle.spawn` 跑在**调用 `execute` 时的那个 tokio runtime** 上（`engine.rs` 约 803 行）。生产 serve 是 `new_multi_thread`（`src/serve/run.rs`）。`current_thread` 出现在测试和若干 session 路径。死锁条件仍然成立：在这条宿主 future 里调用 `AgentJobRegistry::wait`（`src/terminal/agent_jobs.rs` 约 429 行，`Condvar`，占住 worker）会堵住同一 runtime 上的预算监视（`spawn_watcher`）和子进程 `wait`。禁止工具路径调用 `jobs.wait`。
2. **两条超时语义不一样。** Direct `bash`（`src/tools/bash.rs` 约 104–139 行）超时后进程继续跑，并返回 running + `bash_id`（上限 600 秒）。`run_command`（`src/tools/run_command.rs` 约 386–408 行）超时会 `kill_process_group` 并 reject `timeout`（上限 120 秒）。半成品是后者。本计划改成 promote。
3. **explore SAFE 不是「所有 cargo 都拒绝」。** `READONLY_PREFIXES`（`src/tools/bash_safety.rs` 约 20–31 行）包含 `cargo check` / `cargo test` / `cargo clippy`，SAFE 会放行。`cargo build` / `cargo run` 不在表里，默认 Deny。危险命令 floor 对 `run_command` 仍先于策略。
4. **退出邮箱不直接挂在 registry 上。** `src/runtime/exec.rs` 的 `append_seam_reminders` 与 `restore_bash_mailbox`（约 278–288、577–587 行）只通过 `Tool::agent_terminal()` 找 hub。实现这个方法的只有 `bash` / `wait_shell` / `kill_shell`。删掉这三个工具后，邮箱取不到，`reminder/bash_exit` 会静默消失。Phase 1 改成 `runtime_handle.ide.terminal.jobs`（`RuntimeHandle.ide` 已有，`src/ide_base.rs` 的 `terminal`）。不要留一个空工具专门实现 `agent_terminal`。
5. **快照路径写死了 bash 目录。** `output_file_rel`（`src/terminal/agent_jobs.rs` 约 20–26 行）总是拼 `.litecode/bash/{文件名}`。管道日志放在 `.litecode/run_command/` 时，面板会指错文件。
6. **`display_output_path` 在将被删除的文件里。** `src/tools/bash_status.rs` 约 7–19 行，`exec.rs` 在用。先挪走再删格式化函数。
7. **有一条测试和种子矛盾。** `src/config/global_db/seed.rs` 的 `strip_default_direct_shell_bindings` 已从 default/explore/general/orchestrator 删掉三件套，同文件测试也断言它们不在。`tests/global_db_seed.rs` 约 121 行仍断言 default 含 `wait_shell`。Phase 0 改测试，禁止把 `wait_shell` 加回种子。
8. **顶层工具名测试还停在投影之前。** `src/tool/registry.rs` 的 `core_bindings_produce_full_builtin_list` 断言扁平列表含 `run_command`。投影之后模型看到的是 `run_code`，`run_command` 只在 SDK 白名单里。不要为了这条断言把 `run_command` 留在 Direct 列表。

## 1. 协作

| 角色 | 做什么 | 不做什么 |
|---|---|---|
| 规划（本文，已完成） | 冻结契约、阶段、门禁 | 不写产品代码 |
| 实现 | 只做当前阶段列出的文件 | 不提前做下一阶段的行为；不提交 |
| 独立对抗审查 | 每个阶段换一个**不是该阶段作者**的人，按「要打坏什么」动手 | 不替实现改代码来让门禁变绿；不跳过门禁 |
| 组长 | 看审查结论和门禁输出，决定能否进入下一阶段 | 不审查自己写的阶段；不 commit / push / tag |

一个阶段结束的定义：审查者书面说没有重大问题，**并且**该阶段的门禁命令已经跑过，输出与「预期」一致。重大问题包括：第二条命令路径还在、Direct 名字还会从新会话发出来、已 detach 的作业被嵌套 token、程序结束或用户 Stop 杀掉、detach 后管道停读、`run_in_background` 先返回 `job_id` 再把退出丢掉、`jobs.wait` 出现在工具 future 里、`call_id` 进了路径、stdout/stderr 被合并却没有写成决定、explore 用 `run_in_background` 绕过 SAFE。

阶段之间工作区可以是红的。最后一阶段结束时 `cargo check --tests --bins` 与 `web/` 下 `npx tsc -b` 必须是绿的。未经 Andy 要求，全程不提交。

## 2. 非目标

- 人类交互终端不动：`TerminalHub` 的交互 `PtySession`、`terminal/*` WS、dockview 里 shell 标签为 `bash` 的那个面板（那是 shell 程序名，不是工具名）。
- 不克隆 DeepSeek 的 sandbox、提权、read-cursor。只对齐生命周期：进程仍活着才 detach 并返回 job id、前台超时 promote、之后读/杀、退出唤醒。已经退出的不 detach。
- 不做 MCP / custom tool 的 code 化。
- 不改 JS runtime 去保住未返回的 `await`。
- 不 commit、不 push、不打 tag、不升版本。不改 `AGENTS.md`、`docs/ptc/charter.md`。

## 3. 目标契约

### 3.1 进程归谁

**决定：代理命令是管道 `tokio::process::Child`，登记在 `AgentJobRegistry`，不进入 `TerminalHub.sessions`。**

原因：

- `run_command` 的 data 已经把 stdout / stderr 拆开（`RunCommandData`）。`TerminalHub::spawn_command`（`src/terminal/mod.rs` 约 575 行）把 PTY 合成一条 `BoundedTee`，写到 `.litecode/bash/<id>.output`。复用它会丢掉分流。本计划保持分流。
- `TerminalHub::kill`（约 687 行）要求 `SessionOwner::Agent` 的 `PtySession`。没有 PTY 行时 `finish_job_without_pty` 只封 registry，不杀管道进程。UI 的 `bash/kill` 今天走 `kill_from_ui`（`src/client_protocol/workspace.rs` 约 393 行），不能直接拿来杀管道子进程。
- 人类 PTY 与代理作业共用 session 表时，kill / reaper 会打错对象。

保留并继续用的 registry 能力：`insert` / `finish` / `finish_user_killed` / `mark_user_kill`、mailbox、`take_mailbox` / `restore_mailbox`、`set_exit_handler`（已接到 `src/runtime/bash_auto_turn.rs`）、`wire_snapshot`（`src/serve/state.rs` 约 158 行推 `BashJobs`）、`begin_wait` / `end_wait`、`running`。

拆掉的：工具路径上的同步 `wait`；用 PTY 当代理进程主人。`TerminalHub::spawn_command` 与 `TerminalHub::exec_once` 在 Phase 1 删除。Phase 0 之后它们的调用者还在，不只是「专测这两个方法」的测试：`src/runtime/bash_auto_turn.rs` 的 `spawn_echo`（约 262 行）和 `ui_kill_while_busy_prepares_user_stopped_reminder_once_idle`（约 365 行）、`src/tool/executor.rs` 的 `mailbox_stays_pending_for_the_next_seam`（约 1758 行）与 `wait_shell_exit_does_not_wrap_reminder`（约 1776 行）、`tests/terminal_pty_contract.rs` 的 `exec_once`（约 122、171 行）和 `spawn_command_background_writes_output_file_and_kill`（约 202 行，约 218 行调用 `hub.spawn_command`）、`src/terminal/mod.rs` 的 `exec_once` 测试（约 920、957 行），以及 `tests/terminal_ws_isolation.rs` 里的 `TerminalHub::spawn_command`。`spawn_command_background_writes_output_file_and_kill` 只覆盖将被删除的 PTY agent spawn，与 `TerminalHub::spawn_command` 一起删。Phase 1 改写或随方法删除这些调用。`src/terminal/pty.rs` 约 319 行是 `portable_pty` 的 `slave.spawn_command`，不删。`pty::spawn_interactive` 留下。

监管任务（supervisor）从工具 future 里 `tokio::spawn` 到**当前 tokio runtime**（也就是 `ptc` 抓到的那个 Handle），禁止在 `ptc-js` 上跑。它拥有 `Child`（保留 `kill_on_drop(true)`）、两边管道、pid。`bind_child_to_parent` 继续绑。工具 future 只等 supervisor 发出的那一次 oneshot。

嵌套调用的 cancel 不是私有的。`invoke_binding` 传入 `cancel.child_token()`，父级是 `ExecuteRequest.cancel`（`run_code` 的 execution token），再往上是 turn token。`run_code` 的 token 被 drop 时，tokio-util 会把还活着的 child token 重新挂到 turn token 上。用户 Stop 走 `src/session/manager.rs` 的 `cancel_locked` → `live.cancel.cancel()`。`Ledger::cancel_open` 只取消尚未完成的槽，所以「返回 `background` 后程序正常结束」可以是绿的，Stop 仍会杀掉作业。

因此 supervisor **只在 `detached == false` 时** `select` 这颗嵌套 token，以及从它派生的 token。把 `detached` 设为 true 的那一步同时 drop 这颗 token 及其全部派生 token。stdout/stderr 的读任务用 supervisor 自己的 token，一直读到 EOF。禁止把嵌套 token 传给今天 `drain_pipe` 那种「cancel 就停读」的循环：detach 之后停读，管道缓冲写满，子进程堵在 write 上，邮箱永远等不到退出。

`detached` 只由 supervisor 设置，并且和「这次 oneshot 发哪一种结果」是同一步。子进程已经退出：发 `exited`，不 detach，不进 mailbox（结果在这次工具返回里）。否则先把 `detached` 设为 true、丢掉嵌套 token，然后才发 `background` 或 `promoted`。`run_in_background: true` 走同一判断，不跳过退出分支。工具 future 禁止在 oneshot 到达之后再设 `detached`。未 detach 的 `finish` 把退出交给工具 oneshot，不进 mailbox、不叫 exit_handler。已经 detach 之后的 `finish` 才进邮箱并触发 idle auto-turn。

### 3.2 `run_command`（已有名字，Code）

输入：

- `command`：非空字符串，必填。
- `run_in_background`：布尔，默认 false。为 true 时忽略 `timeout_sec`。
- `timeout_sec`：整数 1–120，默认 30。只限制这次前台等待。

输出用 `oneOf`（`src/tool/code_manifest.rs` 已能渲染 `oneOf` / `const`），`kind` 替换现在的 `status`：

| kind | 何时 | 字段 |
|---|---|---|
| `exited` | supervisor 做 detach 判断时进程已经结束。含很快的 `echo`，即使调用方传了 `run_in_background: true` | `exit_code`（integer 或 null）、`stdout`、`stderr`、可选 `output_file`、可选 `stderr_file` |
| `background` | `run_in_background: true`，且 supervisor 确认进程仍活着、已经 detach 之后 | `job_id`、两边快照、`output_file`、`stderr_file`（两个文件都留） |
| `promoted` | 前台等到 `timeout_sec`，且 supervisor 确认进程仍活着、已经 detach 之后 | 与 `background` 相同 |

`stdout` / `stderr` 仍是 `{head, tail, bytes, truncated}`，预算继续用 `INLINE_HEAD` / `INLINE_TAIL` / `INLINE_FULL` / `FILE_MAX`。

语义：

- 非零 `exit_code` 仍然 resolve，不 reject。
- spawn 前或等待中取消，且尚未 detach：杀进程树，reject `ToolError` `cancelled`，不登记，不进邮箱。
- 前台超时：supervisor 确认进程仍活着后先 detach，再 resolve `kind: promoted`。进程继续跑。这是相对今天 `run_command` 杀进程的行为变更。理由：目标 2 要求作业能在之后的轮次被退出唤醒；杀掉就没有可唤醒的进程。Direct `bash` 今天就是「超时变后台」。若超时判断时进程已经退出，发 `exited`，不 detach。120 秒上限只封前台等待，避免一次 `await` 吃掉 `run_code` 的默认预算（`DEFAULT_RUN_TIMEOUT_MS = 120_000`，`src/tools/code.rs`）。后台作业没有这 120 秒寿命上限。
- `run_in_background: true` 不单独走「spawn 成功就返回 job id」。它和 promote 使用 supervisor 的同一步判断：已退出则 `exited`；仍活着才 detach 再发 `background`。这样很快的 `echo` 不会留下一个没有 `reminder/bash_exit` 的 `job_id`。
- 若 `run_code` 的预算先到，I7 取消这次尚未返回的调用，走取消路径（杀进程）。job id 没返回，作业不存活。描述里写明：长于数秒的命令应传 `run_in_background`，不要指望 promote 一定赢过程序预算。用户 Stop 取消的是 turn token；detach 同一步已经丢掉嵌套 token 之后，Stop 不杀这个作业。
- cwd 永远是 `execution.workspace_root`。没有 `workdir`。环境变量保持 `GIT_PAGER=cat`、`PAGER=cat`、`GIT_TERMINAL_PROMPT=0`。shell 用 `shell_command`。
- `Tool::timeout()` 继续返回 `None`，避免执行器再套一层杀进程的钟。
- 权限、危险命令 floor、只读并发、写锁与今天的 `run_command` 相同，且发生在 spawn 之前（I5）。`run_in_background: true` 不能绕过 SAFE 或 floor。

### 3.3 `command_output` / `command_kill`（新，Code）

两个都是 Code 工具，给 `run_code` 调用。禁止留 Direct `wait_shell` / `kill_shell` 当暗门。

它们**不**单独出现在设置卡片里，**不**恢复 `BASH_SERIES`。`build_tool_list` 在该调用方可见 `run_command` 时，把这两个工具追加进同一批，再交给 `project_tools`。关掉 `run_command` 则 SDK 里三个都没有。名字加入保留字，custom / MCP 不能占用 `command_output`、`command_kill`，也不能占用已退役的 `bash` / `wait_shell` / `kill_shell`。

`command_output` 输入：`job_id` 必填；`wait_sec` 整数 0–120，默认 0（立刻快照）。等待用 `wait_async`，并 `begin_wait` / `end_wait`，让现有 `BashJobsSnapshot.waits` 还有意义。

- 已退出：`kind: exited`，并 `take_notice`，避免同一退出再变一条 `reminder/bash_exit`。
- 等到退出：同上。
- `wait_sec` 用尽仍活着：`kind: running`，作业继续，不杀，不再 promote。
- 等待中被 I7 / 用户取消：reject `cancelled`，**不**杀作业（job id 早已返回，归属在 registry）。
- 未知 id，或 id 属于别的 session：`not_found`。错误文本可以列出**本 session** 仍在跑的 id，不列出别的 session。

`command_kill` 输入：`job_id`。对本 session 的作业：进程树杀掉，`kind: killed`，带 `exit_code`、`output_file`、`stderr_file`。这是代理自己杀的，`take_notice`，不再发「用户停了」提醒。未知 id：`not_found`。不碰人类 PTY id（不在 registry 里）。

UI 的 `bash/kill` 是人点的：`mark_user_kill` + 杀进程树 + `finish_user_killed`，**不** `take_notice`，邮箱留给 `reminder/bash_exit`（`killed: true` 的现有文案）。

两个工具的权限是 Fixed allow。它们不重新跑命令策略。能读能杀的前提是这个 session 已经通过 `run_command` 的权限把作业建出来。跨 session 的 id 一律 `not_found`。

### 3.4 取消、I7、唤醒

- 未 detach 的 `run_command`：嵌套 token 取消 = 杀进程树。这一步还选得中这颗 token。
- detach 的同一步丢掉嵌套 token 及其派生 token。之后程序结束、`run_code` token 被 drop、以及用户 Stop（turn token）都不取消 supervisor，也不让 stdout/stderr 停读。管道读到 EOF。
- `command_output` 的等待：取消 = 结束这次等待，进程留着。这颗 cancel 只属于这次等待，不传给 supervisor，也不传给管道读任务。
- 唤醒沿用现成链路：detached `finish` → mailbox → `install_idle_auto_turn`。会话正忙则 `SkippedBusy`，`TurnFinished` 时再冲。下一轮请求缝写 `reminder/bash_exit`（kind 不改）。`command_output` 已消费的通知不再进提醒。
- 提醒结构化字段 `output_file` 继续指向 **stdout** 日志。正文多一行 `stderr_file:`。不新增必填字段，旧行仍能解码。

### 3.5 文件名与权限

- job id：`bg_` + 8 hex（与 `TerminalHub::next_agent_id` 相同字符集）。碰撞时重试 registry 与两个日志文件。
- 路径：`<workspace>/.litecode/run_command/{job_id}.stdout` 与 `.stderr`。禁止把 `call_id` 放进路径。
- 前台 `exited` 且两边都小：删掉两个临时文件，`output_file` / `stderr_file` 省略（与今天一致）。`background` / `promoted` 总是留文件。
- `bash/tail` 的 `tail_view` 读 **stdout** 的 `BoundedTee`。面板因此有活输出。stderr 在工具 data 和 `.stderr` 文件里。这是面板范围，不是把流合并掉。
- explore SAFE：只读命令 Allow，其余 Deny。`cargo check|test|clippy` 按现有前缀放行；`cargo build` 拒绝。floor `floor_dangerous_command` 只挂在 `run_command` 上。Phase 4 从 floor / preset / `authorize` / messages 的工具名分支里去掉 `bash`，规则种类 `BashReadonlyCommand` 保留。
- 写锁只覆盖工具调用本身，不覆盖 detach 之后的进程寿命。这与今天 Direct `bash` 返回 running 之后锁就放开相同。不新做「直到退出还占着 Workspace 写锁」，否则会卡死会话。

### 3.6 人怎么看见活作业

D8：嵌套结果在 `run_code` 那一个 Item 的账本里（序号、工具名、参数摘要、终态），没有单独的 function-call 卡片。人看活作业的地方是会话快照面板：`BashJobs` → `web/src/stores/bashStore.ts` → `SessionStatusLine` 的 `TerminalPanel`。账本行只说明 `run_command` 已结束；job 的命令与尾部输出在面板。

`call_id` 仍可放进 `BashJobWire`（嵌套 id 含 `:`，只做字段，不做文件名）。Reveal：transcript 里没有名为 `bash` 的行时隐藏；若要跳，跳到父调用（去掉后缀 `:code:<数字>` 的那条 `run_code`）。

旧会话里已经落下的 `function_call` 名字 `bash` 继续用 `web/src/components/toolviews/BashToolView.tsx` 回放。`wait_shell` / `kill_shell` 的专用视图删掉，旧 Item 走 `ToolContentView` 的默认输入/输出，不至于空白。新会话不得再发出这三个工具名。

## 4. 保留 / 删除 / 重建

### 保留

- `src/tools/run_command.rs`、`src/tools/descriptions/run_command.md`（Phase 2 改契约，不删）
- `src/tools/bash_safety.rs`、`src/terminal/shell.rs` 的 `shell_command`、`src/terminal/tee.rs`
- `src/terminal/mod.rs` 的 `bash_nonce`、人类 PTY、session 表；`src/terminal/pty.rs` 的交互 spawn
- `src/terminal/agent_jobs.rs` 的表、邮箱、快照、exit handler（等待与杀进程按 Phase 1 改）
- `src/runtime/bash_auto_turn.rs`、`src/reminder/sources/bash.rs`、kind `reminder/bash_exit`
- `src/serve/state.rs` 的 jobs_changed → `BashJobs`
- `src/proc_lifetime.rs` 的 `bind_child_to_parent`
- `web/src/stores/bashStore.ts`、`BashJobs` 协议、状态栏活作业、RPC `bash/tail` 与 `bash/kill`
- `BashToolView.tsx`：旧 `bash` 卡片回放 + 面板尾部。dockview 终端面板（shell 名 `bash`）
- 权限匹配器 `BashReadonlyCommand`

### 删除

- `src/tools/bash.rs`、`wait_shell.rs`、`kill_shell.rs`、`bash_jobs_contract.rs`
- `src/tools/bash_status.rs` 里除 `display_output_path` 以外的格式化（该函数先挪到 `src/terminal/mod.rs` 或 `src/reminder/`）
- `src/tools/descriptions/bash.md`、`wait_shell.md`、`kill_shell.md`
- `src/tools/mod.rs` 里对应 `mod`
- `src/tool/registry.rs` 里三件套的构造臂
- `src/config/global_db/tools.rs` 的 `direct_shell_tools`
- `src/config/schema.rs` 的 `BASH_SERIES_TOOL_IDS` 以及 `core_tool_enable_series` 里的 bash 系列（subagent 系列留下）
- `web/src/api/settings.ts` 的 `BASH_SERIES_TOOL_IDS` 及只为这三项服务的串联测试
- `web/src/components/toolviews/WaitShellToolView.tsx`、`KillShellToolView.tsx` 及 `registry.tsx` / `paramMeta.ts` / `ToolIcon.tsx` / `InlineToolRow.tsx` / `toolCategory.ts` 里只为这两个名字存在的分支
- Phase 1：`TerminalHub::spawn_command`、`TerminalHub::exec_once`。调用点按 Phase 1 改写，不是只删「专测 spawn」的文件：`src/runtime/bash_auto_turn.rs`、`src/tool/executor.rs` 的两处邮箱夹具、`tests/terminal_pty_contract.rs` 里的 `exec_once` 测试和 `spawn_command_background_writes_output_file_and_kill`（约 202 行）、`src/terminal/mod.rs` 里的 `exec_once` 测试、`tests/terminal_ws_isolation.rs` 里的 `TerminalHub::spawn_command`。`spawn_command_background_writes_output_file_and_kill` 与 `TerminalHub::spawn_command` 一起删。订阅注水测试改成对 registry 插入管道作业，不删「重连要带上活作业」这条保证（`tests/subscribe_bash_hydration.rs`）。不删 `pty.rs` 的 `portable_pty` `spawn_command`，不删唤醒测试。
- `src/tool/trait_.rs` 的 `agent_terminal`：Phase 1 邮箱改道之后若无调用者，连默认方法一起删

### 重建

- registry：管道 pid、stdout `BoundedTee`、stderr 捕获、`detached`、`wait_async`；删掉对工具可见的同步 `wait`
- `output_file_rel` 改为真实相对路径（`display_output_path`）
- `bash/kill` 按 pid 杀进程树（Windows `taskkill /T /F`，Unix 进程组），不再要求 PTY
- `exec.rs` 邮箱改走 `runtime_handle.ide.terminal.jobs`
- `run_command` 的后台 / promote / 取消
- `command_output`、`command_kill` 与 SDK 注入
- `src/config/guides/readonly.md` 里 `bash/<id>.output` 那一行（该文件经 `src/config/guides.rs` 进模型上下文）
- 面板不依赖名为 `bash` 的 function-call 才能列出活作业

种子：`ensure_core_bindings` 对**所有** agent 删除 `bash` / `wait_shell` / `kill_shell` 行，不只四个内置 id。否则自定义 agent 的旧绑定会指向不存在的工具。

## 5. 阶段

### Phase 0 — 先删 Direct，编译图只留点名的红

范围：删除 §4 的 Direct 工具模块、描述、registry 臂、`direct_shell_tools`、bash 启用系列、web 上只为 `wait_shell` / `kill_shell` 存在的视图与串联。把 `display_output_path` 挪走。`src/tool/executor.rs` 里对已删除类型 `BashTool` / `WaitShellTool` / `bash_status` 的引用要拿掉，否则 `cargo check --tests` 过不了。`mailbox_stays_pending_for_the_next_seam` 与 `wait_shell_exit_does_not_wrap_reminder` 这两个函数留下，里面的 `spawn_command` 也留下。`wait_shell` 不把提醒包进工具结果的断言挪到 Phase 3 的 `command_output`；Phase 0 不删这两个函数。`tests/global_db_seed.rs` 改为断言 default **没有** `wait_shell`，**有** `run_command`。

本阶段不实现后台，不改 `WaitEnd::TimedOut` 的杀进程，不删 `TerminalHub::spawn_command` / `exec_once`。`src/runtime/bash_auto_turn.rs`、`src/tool/executor.rs`、终端契约测试仍会调用它们；Phase 1 再改写并删除方法。

最小保留，不是工具桩：`display_output_path` 必须留下，因为提醒缝在用。不要为了编译保留 `BashTool` 空壳。

原始删除后、清引用之前，`cargo check --tests --bins` 预期只爆这些位置（爆在别处就是计划漏项，审查拒绝）：

- `src/tools/mod.rs`
- `src/tool/registry.rs`
- `src/tool/executor.rs`
- `src/tool/availability.rs`（系列断言）
- `src/config/global_db/tools.rs`、`seed.rs`、`src/config/schema.rs`
- `src/permission/presets.rs`（`direct_shell_tools` 的 import）
- `src/runtime/exec.rs`（`bash_status::display_output_path`）
- `tests/global_db_seed.rs`
- `web/src` 里 import 已删视图 / `BASH_SERIES_TOOL_IDS` 的文件（`npx tsc -b`）

清完这些引用之后的门禁：

```
cargo check --tests --bins
```

在 `web/`：

```
npx tsc -b
```

预期两者都是退出码 0。

审查者必须打开代码确认下面三处红还在（这是本阶段的产品缺口，不是编译错误）：

1. `run_command.rs` 的 `WaitEnd::TimedOut` 仍杀进程组。
2. `exec.rs` 仍用 `agent_terminal()`；三个工具删除后它拿到 `None`，退出提醒暂时接不上。禁止用空工具接回去。
3. 仓库里还没有 `command_output` / `command_kill`。

要打坏什么：

- 新会话或 registry 是否仍能构造出名为 `bash` / `wait_shell` / `kill_shell` 的工具。
- 是否把 `run_command` 改回 Direct 来让 `core_bindings_produce_full_builtin_list` 变绿。
- 是否把 `wait_shell` 加回种子来迁就 `tests/global_db_seed.rs`。
- 人类终端测试（`web/src/dockview/panels/TerminalPanel.test.tsx` 的 `shell: "bash"`）是否被误删。
- `BashToolView` 是否被删掉，导致旧 `bash` Item 没有回放视图。

### Phase 1 — registry 归属 + 异步等待

范围：管道 supervisor 与 `detached`，取消与管道读按 §3.1（嵌套 token 只在未 detach 时进入 `select`；detach 同一步丢掉该 token；管道读用 supervisor 自己的 token 直到 EOF）。`wait_async`（`Notify`，持锁期间不 `.await`）；删除或改为私有且无工具调用者的同步 `wait`；`output_file_rel`；按 pid 杀进程树；`bash/kill` 与 `bash/tail` 走管道作业；邮箱改走 `runtime_handle.ide.terminal.jobs`。`run_command` 仍是前台杀进程，本阶段不接后台。

删除 `TerminalHub::spawn_command` 与 `TerminalHub::exec_once` 时，同时改掉这些仍活着的调用者：

- `src/runtime/bash_auto_turn.rs`：`spawn_echo`（约 262 行）和 `ui_kill_while_busy_prepares_user_stopped_reminder_once_idle`（约 365 行）改为向 registry 插入一条已经结束的管道作业。需要 `user_killed` 的那条用 `finish_user_killed`，不再调用 `spawn_command` / `kill_from_ui` / `close_agent`。`try_begin_idle_auto_turn` 和这些唤醒测试留下，Phase 2 继续用。
- `src/tool/executor.rs`：`mailbox_stays_pending_for_the_next_seam`（约 1758 行）与 `wait_shell_exit_does_not_wrap_reminder`（约 1776 行）的夹具改为插入已结束的管道作业。不删这两个测试。
- `tests/terminal_pty_contract.rs` 约 122、171 行，以及 `src/terminal/mod.rs` 约 920、957 行：`exec_once` 测试和 `exec_once` 一起删。同文件的 `spawn_command_background_writes_output_file_and_kill`（约 202 行，约 218 行 `hub.spawn_command`）与 `TerminalHub::spawn_command` 一起删。它只覆盖 Phase 1 去掉的 PTY agent spawn。
- `tests/terminal_ws_isolation.rs` 里对 `TerminalHub::spawn_command` 的调用改写或删掉该用例，不删文件里人类终端隔离。
- `tests/subscribe_bash_hydration.rs` 改为对 registry 插入管道作业。
- 不删 `src/terminal/pty.rs` 约 319 行的 `portable_pty` `spawn_command`，不删 `spawn_interactive`。

`done_when`：工具、runtime 和 `tests/` 里都没有 `jobs.wait(`，也没有 `TerminalHub::spawn_command` / `exec_once` 的调用。`spawn_command_background_writes_output_file_and_kill` 已删除。门禁是 `cargo check --tests --bins`，集成测试留下这次调用则门禁仍是红的。`pty.rs` 的 `slave.spawn_command` 还在。`wait_async` 在 `#[tokio::test(flavor = "current_thread")]` 里等待时，兄弟任务仍能跑（对照 `run_command` 已有的 `wait_yields_so_budget_watcher_can_run`）。`bash/kill` 对管道 pid 使用与 `run_command::kill_process_group` 相同的 Windows `taskkill /T /F` 与 Unix 进程组，且拒绝人类 PTY id。`bash_auto_turn` 的唤醒测试仍然存在并通过。

门禁：

```
cargo test --lib terminal::agent_jobs -- --test-threads=1
cargo test --test subscribe_bash_hydration -- --test-threads=1
cargo check --tests --bins
```

预期全绿。

要打坏什么：

- 在 current_thread 测试里把 `wait_async` 改成直接 `jobs.wait`，兄弟任务应被卡住；实现若如此，审查拒绝。
- `spawn_blocking(jobs.wait)` 也拒绝：取消要靠 `select`，不要 50ms 轮询占着 blocking 池。
- supervisor 在 detach 之后仍 `select` 嵌套 token 或 turn token；管道读绑在嵌套 token 上，cancel 后停读。
- 为了让 `cargo check --tests` 变绿而删掉 `try_begin_idle_auto_turn` 的测试，或删掉 `executor.rs` 的两个邮箱测试。
- 删 `pty.rs` 约 319 行的 `portable_pty` `spawn_command`，或删 `spawn_interactive` / 人类终端 WS。
- `bash_auto_turn.rs`、`executor.rs` 或 `tests/terminal_pty_contract.rs` 的 `spawn_command_background_writes_output_file_and_kill` 在门禁时仍调用 `TerminalHub::spawn_command`。
- `finish_job_without_pty` 只改 registry、不杀 pid。
- 快照 `output_file` 仍指向 `.litecode/bash/`。

### Phase 2 — `run_command` 后台、promote、唤醒

范围：`RunCommandTool` 持有 `Arc<AgentJobRegistry>`（从 `ide.terminal.jobs` 构造）。实现 §3.2。更新 `descriptions/run_command.md` 与 data schema（`kind` 替换 `status`）。前台小输出仍删临时文件。

`done_when`：

- 仍在跑的命令：`run_in_background: true` 由 supervisor 先 detach，再返回 `kind: background` 与 `job_id`，进程在返回后仍活着。很快的 `echo` 在判断时已经退出：返回 `kind: exited`，不 detach，不留下没有提醒的 `job_id`。
- 前台短命令返回 `kind: exited`；退出码 7 仍然 resolve，stderr 在 `stderr`。
- `timeout_sec` 到点且进程仍活着：返回 `kind: promoted`，进程仍活着；之后自然退出会进入 mailbox。判断时已经退出则返回 `exited`。
- 未返回前取消：进程树死掉，没有 registry 行，没有邮箱。
- marker：返回 `kind: background` 之后取消 **turn token**（`cancel_locked` 那条，不只是 `run_code` 的程序预算）。进程仍活着，管道继续读到 EOF；之后自然退出仍进入 mailbox。
- `call_id` 含 `:` 时日志文件仍能创建。
- 一个 `current_thread` 测试证明等待期间兄弟任务能跑。

门禁：

```
cargo test --lib tools::run_command -- --test-threads=1
cargo test --lib runtime::bash_auto_turn -- --test-threads=1
```

预期全绿。唤醒测试用现有 `try_begin_idle_auto_turn`：mailbox 有通知且会话空闲、有订阅者时得到 `Prepared`。

要打坏什么：

- promote 或 `run_in_background` 之后丢 `Child`（`kill_on_drop` 把进程杀掉）。
- 工具 future 在 oneshot 到达之后才设 `detached`。很快的 `echo` 加上 `run_in_background: true` 得到 `job_id`，mailbox 却永远空。
- 超时与退出竞态返回 `promoted` 但进程已经没了。
- 只测「程序正常结束」，不取消 turn token。返回 `background` 后调用 turn token 的 `cancel()`，进程必须还在，并且稍后的自然退出必须进 mailbox。短 `timeout_ms` 的 `run_code` 在 `run_command` 还在 `await` 时结束，进程必须死。用 marker 文件断言，不要只看返回 JSON。
- detach 之后管道读因嵌套 token 取消而停住，子进程堵在 write 上，mailbox 没有退出。
- explore 策略在 spawn 之前：SAFE 下 `run_in_background` + `cargo build` 必须 Deny，且没有子进程。
- 把 `timeout_sec` 上限抬到 600，让一次前台 `await` 盖住整个默认程序预算。

### Phase 3 — Code 面的读与杀

范围：`command_output`、`command_kill`，§3.3。注入 SDK 的条件是该调用方有 `run_command`。Direct 调用得到与 `read` 相同的路由拒绝（I2），不执行。

`done_when`：同 session 能读到 running 与 exited；`wait_sec: 0` 不阻塞；等待中取消不杀进程；`command_kill` 杀进程树并 `take_notice`；UI `bash/kill` 留下 `user_killed` 通知。别的 session 的 id 是 `not_found`。设置目录里没有这两张卡片。custom 工具不能起这两个名字，也不能起 `bash`。

门禁：

```
cargo test --lib tools::command_output -- --test-threads=1
cargo test --lib tools::command_kill -- --test-threads=1
cargo test --lib tool::registry -- --test-threads=1
```

模块路径以实现时的 `mod` 名为准；若测试在 `run_command` 子模块，把门禁改成对应过滤器，并在审查记录里写明实际命令。预期全绿。

要打坏什么：

- 只把读/杀做成 Direct 工具，或只在描述里写「用 `wait_shell`」。
- `command_output` 内部调用同步 `wait`。
- 杀掉作业后 mailbox 再推一条重复退出，或 UI Kill 被 `take_notice` 吃掉，人停了作业模型却收不到。
- 跨 session 读到输出。
- 准入失败时工具从 SDK 消失但 `run_code` 仍运行且不报拒绝（I4）。

### Phase 4 — SDK、种子、教学、权限名字

范围：`run_command.md` 教模型三种 `kind`、非零退出、`run_in_background`（进程仍活着才是 `background`；已经退出则 `exited`，不发没有后续提醒的 `job_id`）、`command_output` / `command_kill`、长命令不要靠前台超时。`src/config/guides/readonly.md` 改为 `run_command/{job_id}.stdout|stderr`。floor / preset / `authorize` / messages 的工具名只留 `run_command`。`src/session/DATA.md` 写明新会话不发 Direct 三件套，旧 Item 仍可回放。registry 测试改为断言模型列表含 `run_code`、不含 `bash`/`wait_shell`/`kill_shell`，白名单含三个命令工具。所有 agent 的旧三件套绑定在 `ensure_core_bindings` 删除。

`done_when`：对一个绑了 `run_command` 的 agent，`run_code` 描述里的 `declare const tools` 含 `run_command`、`command_output`、`command_kill`，且不含 `bash`。explore 的存储策略仍是 SAFE（默认 Deny + `readonly_command`）。

门禁：

```
cargo test --lib tool::code_manifest -- --test-threads=1
cargo test --lib permission::presets -- --test-threads=1
cargo test --lib permission::floor -- --test-threads=1
cargo test --lib config::global_db::seed -- --test-threads=1
cargo test --test global_db_seed -- --test-threads=1
```

预期全绿。

要打坏什么：

- SDK 手写第二份描述，和 schema 不一致（I11：描述是投影的纯函数）。
- 教学仍写「超时会杀进程」或「用 `wait_shell`」。
- 删 `bash` 别名时把 `BashReadonlyCommand` 或危险命令 floor 一起删掉。
- 只清四个内置 agent，自定义 agent 的 `bash` 绑定还在。

### Phase 5 — 快照面板在没有 Direct 卡片时仍可用

范围：`TerminalPanel` 只靠 `bashStore` 的 jobs 列出活作业，不要求 transcript 里有 `name === "bash"` 的调用。`bash/tail` 显示 stdout 尾部。可选：`BashJobWire` 增加可缺省的 `stderr_file`，面板副标题能看到。Reveal 按 §3.6。删掉仅用于解析 `wait_shell` / `kill_shell` 卡片的测试；保留 `BashToolView` 对旧 `bash` 结果文本的测试。

`done_when`：没有 bash function-call 时，快照里的一条 `bg_` 作业仍出现在状态栏，Kill 发出 `bash/kill`。旧 `bash` Item 仍走 `BashToolView`。`wait_shell` / `kill_shell` 不在 `TOOL_VIEWS` 里。

门禁：在 `web/`：

```
npx tsc -b
```

预期退出码 0。本仓库没有浏览器工具要求；`tsc` 是本阶段的编译门禁。面板行为由审查者读 `SessionStatusLine.tsx` 的数据来源核对，并跑已经覆盖 `bash/kill` 的现有单测（`SessionStatusLine.test.tsx`、`ToolCallCard.test.tsx` 里若仍假设活卡片来自工具名 `bash`，改成快照驱动，不要留一个只有 Direct 卡片才能 Kill 的路径）。

要打坏什么：

- 面板仍用 `bashCallMetaByCallId` 过滤 `name === "bash"`，嵌套作业因此不出现。
- Reveal 用含 `:` 的嵌套 `call_id` 去查 DOM，静默失败却还显示按钮。
- 为了面板去给 `run_code` 做一张专用卡片（章程：消息列表不对 `run_code` 做专门视图）。
- 删 `BashToolView` 导致旧会话空白。

### Phase 6 — 能力对等测试收口

把 §6 里每一条「新路径」写成测试或标成有意删除。门禁：

```
cargo test --lib tools::run_command -- --test-threads=1
cargo test --lib terminal::agent_jobs -- --test-threads=1
cargo test --lib runtime::bash_auto_turn -- --test-threads=1
cargo test --lib permission::floor -- --test-threads=1
cargo check --tests --bins
```

在 `web/`：`npx tsc -b`。预期全绿。

审查者对照 §6 逐条要证据（测试名或「有意删除」）。没有证据的条目是重大问题。

## 6. 能力对等

| 今天 Direct `bash` | 新路径 | 备注 |
|---|---|---|
| 前台结构化结果 | `run_command` `kind: exited` | 保留分流；字段名 `status` 改为 `kind` |
| 非零退出仍成功返回 | `exited` + `exit_code`，不 reject | 保留 |
| 取消 | 未 detach：杀进程树并 `cancelled` | 保留 |
| 前台超时（今天是变成后台，上限 600 秒） | 上限 120 秒后 `kind: promoted`，进程继续 | 有意收紧上限，避免盖住默认 `run_code` 预算 |
| `run_in_background` | 进程仍活着：`kind: background` + `job_id`。判断时已退出：`kind: exited`，不 detach | 很快的 `echo` 把结果放进这次返回，不留一个没有提醒的 job |
| `wait_shell` 按 id 等 / 读 | `command_output` | Code 面 |
| `wait_shell` 不带 id（睡或任一作业退出） | 无 | 有意删除。程序内没有计时器；退出唤醒走下一轮邮箱 |
| `kill_shell` | `command_kill` | Code 面 |
| 人在卡片上 Kill | RPC `bash/kill`，`user_killed` 提醒 | 保留，改杀管道 pid |
| `reminder/bash_exit` + idle auto-turn | 同一邮箱与 `bash_auto_turn` | kind 不改；缝改走 `ide.terminal` |
| 危险命令 floor | 仍在 `run_command` spawn 前 | 工具名分支去掉 `bash` |
| 只读命令可并行 | `is_concurrency_safe` / SAFE 的 `readonly_command` | 保留，含现有 cargo check/test/clippy 前缀 |
| cwd = 工作区根 | 不变 | 无持久 shell，今天也没有 |
| Windows 进程树 | `taskkill /PID /T /F`；Unix 进程组 | 与现 `run_command` 相同，supervisor 持有 pid |
| 日志可被 `tools.read` 翻页 | `.litecode/run_command/{job_id}.stdout|.stderr` | 目录从 `bash/` 改来；8 hex，无 `call_id` |
| 旧 transcript 里的三件套卡片 | `bash` 用 `BashToolView`；另外两个走默认卡片 | 新会话不发这些名字 |

今天 `run_command` 超时杀进程：有意废除，改为 promote。

## 7. 风险

- **current_thread 死锁。** 在嵌套工具 future 里调用 `AgentJobRegistry::wait` 会占住驱动 `run_code` 宿主调用的 tokio worker。预算监视与子进程 `wait` 都在这个 runtime 上。只允许 `wait_async`。Phase 1 审查要 grep `jobs.wait(`。
- **未 detach 时被取消，作业该死。** 嵌套 token 在 `detached == false` 时仍进入 `select`，取消会杀进程树。detach 的同一步必须丢掉这颗 token 及其派生 token。用户 Stop 取消的是 turn token；`run_code` token drop 后，活着的 child 会重新挂到 turn token 上。只测「程序正常结束」会漏掉 Stop。管道读必须用 supervisor 自己的 token 直到 EOF，否则子进程堵在 write 上，邮箱没有退出。Phase 2 的 marker 在返回 `background` 之后取消 turn token。
- **detach 判断和退出抢在同一步。** 工具 future 若先决定返回 `background`，supervisor 再看到 `detached == false` 的 `finish`，很快的 `echo` 会留下 `job_id` 且没有 `reminder/bash_exit`。只有 supervisor 在发 oneshot 的同一步决定发 `exited` 还是先 detach 再发 `background` / `promoted`。
- **explore SAFE 与 cargo。** `cargo build` 必须拒绝，包括后台。`cargo check` / `cargo test` / `cargo clippy` 今天就会放行，不要顺手改成全拒绝或全放行。
- **120 秒只是前台等待。** 后台作业可以长过 `run_code`，直到退出、`command_kill`、UI Kill，或进程随父进程的 job object 一起结束（`bind_child_to_parent`）。会话结束今天也不会扫掉 Direct 后台作业；新路径同样不在会话结束时额外扫杀。程序预算若短于 `timeout_sec`，I7 会抢先取消未返回的前台调用，表现为杀进程而不是 promote。
- **写锁不住后台进程。** detach 返回后，别的 `edit` / `write` 可以并行。接受，与今天返回 running 之后相同。
- **面板尾部只有 stdout。** stderr 在 data 与 `.stderr`。审查若发现文件也被合成一个流，这是重大问题。

## 8. 审阅修订

审查者 `e758ba9a-4e6b-46f4-bc68-eb127d4b38cf`，结论 REVISE。只改了本文件。

1. **detach 之后不再持有 turn 上的 cancel。** §3.1、§3.4、Phase 2 的 marker：嵌套 token 只在 `detached == false` 时进入 `select`；设 `detached` 的同一步 drop 该 token 及其派生 token。管道读用 supervisor 自己的 token，直到 EOF。marker 在返回 `background` 之后取消 turn token，进程仍活着，随后的自然退出仍进 mailbox。
2. **退出先于 detach 不再丢掉唤醒。** `detached` 只由 supervisor 在选择 oneshot 的同一步设置。已经退出就发 `exited` 且不 detach；否则先 detach 再发 `background` 或 `promoted`。`run_in_background` 不跳过退出分支。工具 future 不在 oneshot 之后设 `detached`。§3.2 的表、Phase 2 的 `done_when`、§6 对等表已改成同一规则。
3. **Phase 1 的 `cargo check --tests` 能编过。** 范围补上 `bash_auto_turn.rs`、`executor.rs` 的两处邮箱夹具、`terminal_pty_contract.rs` 与 `terminal/mod.rs` 的 `exec_once` 测试、`terminal_ws_isolation.rs`。前两处改成插入已结束的管道作业，唤醒测试不删。`exec_once` 测试随方法删除。`pty.rs` 约 319 行的 `portable_pty` `spawn_command` 不删。Phase 0 写明这两个 executor 测试函数先留下。第二次审阅补上 `tests/terminal_pty_contract.rs` 的 `spawn_command_background_writes_output_file_and_kill`（约 202 行）：§3.1、§4、Phase 1 调用清单都写明它与 `TerminalHub::spawn_command` 一起删。`done_when` 改为 `tests/` 里也不能留下 `hub.spawn_command`，否则 `cargo check --tests --bins` 仍红。
4. **Phase 3、4、6 的门禁改成每个过滤器一条 `cargo test --lib <filter>`。** `--lib` 不能重复。Phase 1、2 原来就是这样。
