# WP-196 Runtime 实现笔记 — `run_command`（切口 B）

> **作者：** Runtime（Grok Bot 会话自动路由；无 picker）  
> **时间：** 2026-10-21 Asia/Hong_Kong（续：Andy 关 Direct bash 默认）  
> **分支 / HEAD：** `feat/tool-contract-ptc`（实现未 commit；工作树有改动）  
> **依据：** `plan.md`；WP-195 `dig-runtime.md` §7；Andy 新口径（默认关 Direct bash 三件套）

## 1. 做了什么

新增 **Code-only** 工具 `run_command`（SDK：`tools.run_command(...)`）：

- 前台一次性 shell 命令；`cwd = ToolExecutionContext.workspace_root`（不读 LAP）
- **async** `tokio::process` + `select!`（取消 / 超时 / `child.wait`）；**不**走 TerminalHub 作业、`jobs.wait`、不登记后台、无 `bash_id`
- 结束 / 取消 / 超时 → **杀进程组**（unix `kill(-pgid, SIGKILL)`；windows `taskkill /T`）
- 成功稳定 `data`；**非零 exit 仍 resolve ok**；取消 → `ToolError` `cancelled`；超时 → `timeout`
- Direct `bash` / `wait_shell` / `kill_shell` **行为未改**

## 2. 契约字段（终稿，供 Tools 教法）

### 入参

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `command` | string | 是 | 一次性 shell 命令（非持久 session） |
| `timeout_sec` | integer 1–120 | 否 | 默认 **30**；到点**杀进程组**并 reject `timeout`（不转后台） |

### 成功 `data`

| 字段 | 类型 | 说明 |
|---|---|---|
| `status` | `"exited"` | 成功路径常量；无 `running` |
| `exit_code` | integer \| null | 信号终止无码时为 null |
| `stdout` | `{ head, tail, bytes, truncated }` | 有界窗：head≤2KiB、tail≤4KiB（与 tee 常量对齐） |
| `stderr` | 同上 | 同上 |
| `output_file` | string? | 截断或超大时给 workspace 相对路径（`.litecode/run_command/<call_id>.stdout`）；同 stem 的 `.stderr` 并存。小结果省略并删临时文件 |

内容字段 `content` 仅短摘要（`status` / `exit_code`），契约以 `data` 为准。

### 错误

| 情况 | `error_code` |
|---|---|
| 取消 | `cancelled` |
| 到点 | `timeout`（`details.timeout_sec` / `output_file`） |
| 坏入参 | `invalid_input` |
| spawn/IO | `io` |

## 3. 超时数值与理由

| 值 | 选择 | 理由 |
|---|---|---|
| 默认 30s | 与旧 bash 前台等待量级同，但语义是**杀**不是转后台 | 短测/构建默认够用 |
| 上限 120s | ≤ 常见 `run_code.timeout_ms` 默认 120000 | 单次调用不能独自吃满整段程序预算；**不做双份**：更长作业拆程序/`timeout_sec` 或人工开 Direct（默认不可见） |
| `Tool::timeout()` | `None` | 只靠入参 `timeout_sec` + 取消令牌；避免第二套工具级钟（对齐 D-09 精神） |

## 4. 取消语义

1. `execution.cancel` 在 `select!` 上与 `child.wait` / sleep 并列 → **current-thread 可调度**预算 watcher（单测 `wait_yields_so_budget_watcher_can_run`）。
2. 取消或超时：先 `kill_process_group`，再 `await child.wait`，再收尾 pipe task。
3. **不**返回可跨调用的作业 id；程序结束即无残留登记（kill_on_drop 作兜底）。
4. 与 Direct bash「到点转后台 / 取消杀前台」刻意分离。

## 5. effects / 并发 / 权限

- **effects：** 只读命令（`is_readonly_command`）→ 空；否则 `ToolEffects::write(ResourceKey::Workspace)`（Q5 最小「可能写 workspace」）。
- **resource_keys：** 复用 `extract_bash_paths`（与 bash 同），供 tracker；锁靠 effects。
- **is_concurrency_safe：** 只读命令 true。
- **is_destructive：** 复用 `is_destructive_command`。
- **floor：** `bash | run_command` 共用 `floor_dangerous_command`（未改规则逻辑，仅覆盖新工具名）。
- **preset：** 与 bash 同（SAFE：默认 Deny + `readonly_command` Allow；ALL：allow-all）。
- **未改** Direct bash 教法 / WP-194 文案目标。

## 6. 测了什么 / 未跑 cargo

单测草稿在 `src/tools/run_command.rs`（`#[cfg(test)]`）：

| 测 | 覆盖 |
|---|---|
| `surface_is_code_only` / `data_schema_*` / `validate_timeout_bounds` | 契约面 |
| `effects_workspace_write_for_mutating_commands` | effects |
| `success_data_and_nonzero_exit_still_ok` | data 字段 + 非零仍 ok |
| `cancel_kills_process_group` | 取消杀组 |
| `wait_yields_so_budget_watcher_can_run` | current-thread 上等待让出（预算 watcher 类比） |
| `uses_workspace_root_as_cwd` | cwd=workspace_root |

**未跑** `cargo test` / `cargo build` / 全量 check（硬约束；测绿需 mid 放行重型槽）。

**测绿前还须：**

1. mid 放行 cargo 后跑：`run_command` 单测 + 相关准入/投影。
2. **刷新** `src/tool/fixtures/run_code_sdk.{txt,sha256}`（准入后 SDK 文面会变；属 Tools 快照，Runtime 未改 fixture）。
3. Tools：教法 md 可在本 stub 上加厚；`code_manifest` 由投影自动生成。

## 7. 与 Direct bash 边界（一句）

`run_command` = Code 内**唯一默认**短前台可取消结构化执行。Direct `bash`/`wait_shell`/`kill_shell` 实现保留，但**默认对 agent 不可见**（见 §10）；不做双轨教法、不教两套。

## 8. 改动文件清单

| 路径 | 变更 |
|---|---|
| `src/tools/run_command.rs` | **新建** 实现 + 单测 |
| `src/tools/descriptions/run_command.md` | **新建** 最小 stub 描述（准入必需） |
| `src/tools/mod.rs` | `pub mod run_command` |
| `src/tool/registry.rs` | 注册 `run_command`；默认绑定测改为无 bash / 有 run_command |
| `src/config/global_db/tools.rs` | `run_command` 入 `core_configurable`；三件套移出并加 `direct_shell_tools` |
| `src/config/global_db/seed.rs` | 默认不绑三件套；ensure DELETE 残留；seed 测更新；revision 16 |
| `src/permission/presets.rs` | SAFE/ALL 对齐 bash；`apply_preset` 仍覆盖 direct_shell（逃生） |
| `src/permission/floor.rs` | 危险命令 floor 覆盖新名 |
| `src/permission/messages.rs` | 拒绝文案覆盖新名 |
| `src/tool/authorize.rs` | permission summary |
| `.litecode/ops/artifacts/WP-196/dig-runtime-impl.md` | 本笔记（含 §10 关闭层） |

## 9. 未 commit / 需 mid

- **未 commit、未 push。**
- mid 已放行独占测；**不**自行刷 `run_code_sdk.*` fixture（Tools 管；工作树 sha 已由 Tools 更新）。
- 契约字段已稳定；Tools 单轨教法只推 `run_command`。
- explore / general / default / orchestrator：**默认绑 `run_command`，不绑 Direct bash 三件套**（见 §10）。

## 10. Direct bash 默认关闭层（Andy 口径）

**关闭落点：`agent_tools` 种子 / ensure（agent 绑定层），不是删 registry，也不是改 ToolSurface。**

| 层 | 行为 |
|---|---|
| `core_configurable_tools()` | **已去掉** `bash` / `wait_shell` / `kill_shell`；保留 `run_command`。`bind_configurable` / `bind_subagent_configurable` / ensure 补缺循环因此默认不绑三件套。 |
| `direct_shell_tools()` | 新列表；`is_core_tool` / `core_tool_ids` / `apply_preset_to_tools` 仍认识三件套 → 人工绑/逃生可用；preset/floor 规则未删。 |
| `seed_explore_agent_bindings` | 只 upsert `run_command`（SAFE）；**不再** upsert bash/wait/kill。 |
| `strip_default_direct_shell_bindings` | seed + `ensure_core_bindings` 入口：`DELETE` default/explore/general/orchestrator 上残留三件套行（仿 `bash_output`）。自定义 agent 不动。 |
| project 投影 | 只投影已绑定且可见工具 → 默认 agent 视角看不到 Direct 三件套；能看到/绑定 `run_command`（Code 面）。 |
| registry / 实现 | `bash.rs` / `wait_shell` / `kill_shell` 模块与注册保留。 |
| SDK fixture | **Runtime 不刷**；Tools 维护 `run_code_sdk.*`。 |

`SEED_REVISION` → `"16"`。

删掉原「更长作业走 Direct bash」双轨表述；默认路径只有 `run_command`。
