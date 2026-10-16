# WP-188（WP-183-a）fill plan — Obs-1 + Obs-2（runtime）

**board id:** WP-188  
**parent:** WP-183（`.litecode/ops/artifacts/WP-183/plan.md`）  
**brief:** `.litecode/ops/artifacts/WP-183/183-a-brief.md`  
**design 依据:** WP-180 Obs A/B/C + §4（`.litecode/ops/artifacts/WP-180/plan.md`）  
**owner:** runtime · **approver:** backend_mid  
**产物目录:** `.litecode/ops/artifacts/WP-183/183-a/`  
**口径:** observe-only；尽量不改控制流；**不修 H1 codec**；**不改权限桥 / AskOutcome / reject_all / waiting_for_user**（WP-181）。  
**串行:** Obs-1 → Obs-2。重型与 WP-181 互斥（`LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`）；开跑前报 backend_mid。  
**约束:** 独立分支；不 push `ops/team-framework` 除非 Andy 点头。本 fill **只写 plan**；实现另开。

---

## 1. 范围 / 非目标

### 范围（本包落地）

| # | 项 | 来源 |
|---|----|------|
| Obs-1.1 | 进程级 **panic hook** → `crash-<unix>.log`（栈 + 若可得的 session/turn） | WP-183 §1 Obs-1.1 / WP-180 §4.2 |
| Obs-1.2 | **Diagnostics 目录约定** 落地（读写 `data_root`） | WP-183 §1 Obs-1.2 / WP-180 §4.4 |
| Obs-1.3 | **工具调用审计**（observe-only）：append-only 文件 + 固定 tracing target | WP-183 §1 Obs-1.3 / WP-180 §4.3 |
| Obs-2.1 | **TurnError / GiveUp ↔ wire**：日志打出 wire 相对路径（`LITECODE_LLM_WIRE` 已开时） | WP-183 §1 Obs-2.1 |
| Obs-2.2 | **失败保留策略**：文档化；实现以最小扰动为准（默认保留现有 retention，路径已打） | WP-183 §1 Obs-2.2 |
| Obs-2.3 | **Luna + grep 排障 runbook** 写入 artifacts | WP-183 §1 Obs-2.3 / WP-180 §8 |

### 非目标（强制）

- **不修 H1 codec**（unknown `type`、补建 `*_tokens_details`、unknown-event 策略）— 归未来 Fix-H1。
- **不改权限桥 / AskOutcome / reject_all / waiting_for_user / `PhasePermissionSink` 语义** — 归 WP-181（WP-186/187）。
- 不改 FE ErrorBoundary；不强制默认对全体用户开 wire。
- 不借机大改工具控制流；确认既有 `catch_unwind` / pipeline join→error 覆盖即可，只**补审计字段**。
- 不改 `src/session` writer / startup seal 逻辑（diagnostics 只经 `data_root` 文件系统写入）。

---

## 2. 现状（cited paths，工作区只读核实）

### 2.1 工具 panic 隔离（已有，本包只补审计）

| 位置 | 行为 |
|------|------|
| `src/tool/executor.rs:387–411` | `AssertUnwindSafe(...).catch_unwind()`；panic → `tracing::error!(tool=…)` + `ToolCallResult::error("… panicked …")` |
| `src/tool/pipeline.rs:152–163` | 并发 `JoinHandle` `Err(join_err)` → error 结果 + `tracing::error!("tokio task panicked…")` |
| `src/tool/pipeline.rs:249–267` | `join_tool_handle` 同样转 error |
| `src/tool/executor.rs:321–347` | `run_tool` → `authorize(...)` → Deny/Aborted/Proceed；**现无** phase 审计行 |
| `src/tool/authorize.rs:42–110` | Ask 路径 `sink.ask_permission`；`AskOutcome::{Aborted,Deny,Allow}` — **本包不改分支语义** |
| `src/tool/executor.rs:430–435` | 成功路径仅 `tracing::info!(tool, id, output_len, "tool executed")` — 无 start/auth/panic/cancel 统一 schema |

### 2.2 进程入口（panic hook 安装点）

| 位置 | 说明 |
|------|------|
| `src/main.rs:1–3` | `fn main() → litecode::cli::run()` |
| `src/cli/cli.rs:707+` | `pub fn run()`：parse →（CodeSearchWorker / Config 早退）→ `setup_logging`（`:121–156`）→ Serve 或 CLI agent |
| `src/cli/cli.rs:743` | `_log_guard = setup_logging(&workspace.paths.logs_dir)` — **默认在此之后立刻装 panic hook**（日志已可用） |
| `src/serve/run.rs:50` | `serve::run` — Serve 路径仍经 `cli::run`，**不需第二处 hook**（待确认：若未来有旁路 entry，再补） |

**默认选择：** 单点安装在 `cli::run`，`setup_logging` 成功之后、进 Serve/agent 之前；CodeSearchWorker / Config 早退路径可跳过（无 session 诊断需求）。若 hook 需在 logging 前也能写文件，可改为「先解析 paths → 装 hook → setup_logging」——实现时以「crash 文件必落盘」优先。

### 2.3 data_root / session 路径约定

| 位置 | 约定 |
|------|------|
| `src/config/resolved.rs:22–36` | `WorkspacePaths { sessions_db, logs_dir, plan_dir, snapshots_dir }`；`sessions_db = <ws>/.litecode/sessions.db`，`logs_dir = <ws>/.litecode/logs` |
| `src/cli/cli.rs:780–782` | `data_root = db_path.parent()` → 即 **`<workspace>/.litecode/`** |
| `src/session/data/mod.rs:80` / `manager.rs:2623` | `data_root()` / `data_root_path()` |
| `src/session/data/blob.rs:11–12` | `data_root/blobs/` |
| `src/session/media.rs:27–28` | `data_root/media/` |
| `src/runtime/context.rs:22` | `AgentRuntime` 持 `data_root: PathBuf` |

**结论：** diagnostics 落在 `data_root`（`.litecode/`）下，与 blobs/media/logs 并列，**不**写入 `sessions.db`，避免与 WP-181 session writer 抢文件。

### 2.4 LITECODE_LLM_WIRE / wire_dump

| 位置 | 说明 |
|------|------|
| `src/llm/codec/wire_dump.rs` 全文 | env `LITECODE_LLM_WIRE=<base>`；`<base>/<session_id>/`：`summary.jsonl`、`meta.json`、`<n>.<codec>.request.json`、`<n>.<codec>.response.sse.jsonl` |
| `:57–82` | `base_dir()` OnceLock；未设或不可写 → 全 no-op |
| `:97–115` | `session_dir_name`（非法字符 → `_`；空 → `_unknown`） |
| `:52–54` | retention：`KEEP_REQUESTS=400`，`KEEP_SESSIONS=30`（按请求对 / session 文件夹 GC） |
| `:288–350` | `Capture::start(codec, url, session_id, body)` — `pub(super)` |
| `scripts/serve.sh:11,72,282` | `--wire [DIR]` → 设 `LITECODE_LLM_WIRE` |
| `scripts/wire_watch.ps1` | tail `summary.jsonl` |

**缺口（本包 Obs-2）：** TurnError / GiveUp 路径**未**自动打出 wire 相对路径；无「仅失败保留」开关（现一律宽 retention）。

### 2.5 TurnError / GiveUp / on_broken_stream

| 位置 | 说明 |
|------|------|
| `src/runtime/observer.rs:19–22` | `TurnError { reason: FailReason, message }` |
| `src/runtime/observer.rs:11–15` | `FailReason::{LlmHttp, LlmParse, Internal}` |
| `src/runtime/observer.rs:133+` | `InternalEvent::Error(TurnError)` / `TurnCompleted { reason, error_kind, … }` |
| `src/runtime/exec.rs:905–908` | LLM HTTP 失败 → `emit_internal(InternalEvent::Error(TurnError { LlmHttp, message }))` — **无 wire 路径字段** |
| `src/agent/core.rs:58–83` | `call_model` Err → `TurnOutcome::Error`；`LlmStreamInterrupted` 先 persist partial 再 Error |
| `src/runtime/mod.rs:906–909` | `TurnOutcome::Error` → `emit_turn_completed(…, TurnEndReason::Error, …)` |
| `src/llm/codec/http.rs:57–59,190–219,294–318` | `EmptyStreamAction::GiveUp`；`on_broken_stream`（已产 items → GiveUp）；`preserve_partial` → `LlmStreamInterrupted` |
| `src/llm/codec/responses.rs:423+` / `chat.rs:275+` | 各 GiveUp 分支 `return Err(preserve_partial(…))` 或裸 `Err(error)` |

**结论：** GiveUp 最终经 agent → `TurnOutcome::Error` → turn completed(Error)。关联 wire 应在 **runtime 收口**（`finalize_agent_outcome` / `emit_turn_completed` Error 分支，和/或 `exec.rs` Error emit）打结构化日志；codec 层可补一行 `tracing`（带 session_id 若已有），但**不必**改 GiveUp 控制流。

### 2.6 现有 tool tracing

- `executor.rs:409` panic；`:430–435` executed；`authorize.rs:98–101` always-grant。
- `pipeline.rs:153–156,259–261` join panic。
- **无**统一 `phase=` / 无 append-only 审计文件 / 无固定 target（如 `litecode.tool_audit`）。

### 2.7 尚无

- 无 `diagnostics/` 目录；无进程级 `panic::set_hook`；无 thread-local session/turn 供 hook 读取（需本包加轻量 TLS 或等价）。

---

## 3. 实现步骤（Obs-1 → Obs-2，file-level）

### Obs-1 — panic hook + diagnostics + 工具审计

**步骤 O1-1 · 新模块 `src/runtime/diagnostics.rs`（默认路径；待确认也可 `src/telemetry/diagnostics.rs`）**

1. 约定路径 helper（纯函数，可单测）:
   - `diagnostics_root(data_root) → data_root.join("diagnostics")`
   - `session_diag_dir(data_root, session_id) → …/diagnostics/<sanitized_session_id>/`
   - sanitize 可复用 wire 的 `session_dir_name` 规则（复制小函数或抽 `pub(crate)` 到共享处；**默认复制 15 行到 diagnostics，避免动 wire_dump 可见 API 面**）。
2. `write_last_turn(data_root, session_id, LastTurnSnapshot)` → 写/覆写 `last-turn.json`（见 §4）。
3. `append_tool_audit(data_root, session_id, &AuditEvent)` → append `tool-audit.jsonl`（见 §5）；失败只 `tracing::warn!`，**不**影响工具结果。
4. `note_wire_pointer(data_root, session_id)` → 写 `wire-note.txt`（内容含 `LITECODE_LLM_WIRE` base + 相对 session 文件夹名）；**默认不用 symlink**（Windows/权限友好）。若 env 未设，写 `wire: disabled`。
5. 导出 `install_panic_hook(fallback_dir: PathBuf)`（见 O1-2）。

**步骤 O1-2 · Panic hook**

1. `std::panic::set_hook`：在既有 hook 行为上追加写文件（先 `take_hook` 再链式调用原 hook，避免吞掉默认打印）。
2. 文件名：`crash-<unix_secs>.log`（或 `crash-<unix_ms>.log`）；优先写入 **thread-local 当前 session** 的 `diagnostics/<sid>/`；若无 TLS → `diagnostics/_process/` 或 `logs_dir` 旁 `diagnostics/_unknown/`。
3. 内容：panic payload Display、`std::backtrace::Backtrace`（若 `RUST_BACKTRACE`/`force` 可得则完整，否则 best-effort）、TLS 中的 `session_id` / `turn_id` / `step` / `last_tool`（有则写）。
4. **TLS 上下文（最小）:** 在 `src/runtime/diagnostics.rs` 提供 `DiagContext::enter(session_id, turn_id)` guard；在 `AgentRuntime::run_with_turn`（`src/runtime/mod.rs:924+`）入口 set、出口 clear。工具路径靠同一 turn 的 TLS，不强迫每个工具再 set（`last_tool` 由审计 start 更新）。
5. 安装点：`src/cli/cli.rs` `run()`，`setup_logging` 成功后调用 `diagnostics::install_panic_hook(workspace.paths.logs_dir.parent().unwrap_or(logs_dir).to_path_buf())` —— 即默认 fallback = `data_root`（`.litecode/`）。Serve/CLI 共用。

**步骤 O1-3 · last-turn 心跳（observe）**

1. 在 `emit_phase` / `emit_turn_completed` / 工具审计 start·exec_done 时，best-effort 更新 `last-turn.json`（字段见 §4）。
2. 写失败忽略（warn）；**不**引入新 turn 状态机。

**步骤 O1-4 · 工具审计埋点（observe-only，优先改 `run_tool` / pipeline，少碰 authorize）**

文件：`src/tool/executor.rs`（主）、`src/tool/pipeline.rs`（join panic 补字段）。

在 `run_tool` 内：

| 时机 | phase | 备注 |
|------|--------|------|
| 通过 name/schema 校验后、调用 `authorize` 前 | `start` | 必有 tool, call_id, session_id |
| 即将 `authorize` 且后续可能 Ask（或一律在 authorize 前后） | `auth_ask` | **默认：** authorize 调用前打 `auth_ask`；返回后打 `auth_done`（outcome 短标签：`proceed`/`deny`/`abort`，**不**改 AskOutcome） |
| authorize 返回后 | `auth_done` | 同上 |
| catch_unwind 得 panic | `panic` | 与现有 error 结果并行；`ok=false` |
| cancel 早退 / Aborted | `cancel` | |
| finalize 后正常结束 | `exec_done` | `ok` 按 ToolCallResult 是否 error |

`pipeline.rs` join `Err`：补审计 `phase=panic`（或 `exec_done` + `panic=true`）；**默认 `phase=panic`**，与 executor 一致。

**禁止：** 改变 authorize 分支、PermissionSink 调用顺序、cancel 语义、结果替换逻辑。

**步骤 O1-5 · tracing target**

- 每条审计同时：`tracing::info!(target: "litecode.tool_audit", …)`（字段与 JSONL 对齐），便于 `RUST_LOG=litecode.tool_audit=info` 不落盘也能看。
- JSONL 为排障权威（进程死后仍在）。

### Obs-2 — TurnError↔wire + 失败保留说明 + runbook

**步骤 O2-1 · wire 路径解析 helper**

1. 在 `wire_dump.rs` 增加 **最小** `pub(crate) fn wire_session_dir(session_id: &str) -> Option<PathBuf>`（读 `base_dir` + `session_dir_name`）；或在 `diagnostics` 内直接读 `std::env::var("LITECODE_LLM_WIRE")` 拼路径（**默认后者**，零改 codec 可见性；若要单测 wire 开闭，再抽 helper）。
2. 相对路径字符串：`$LITECODE_LLM_WIRE/<session_folder>/`（日志里同时打 abs 与 rel）。

**步骤 O2-2 · TurnError / GiveUp 关联日志**

1. `src/runtime/mod.rs` `finalize_agent_outcome` / `emit_turn_completed` 当 `TurnEndReason::Error`：  
   `tracing::error!(session_id, turn_id, wire_dir = ?…, wire_rel = ?…, error = %msg, "turn error; wire capture path")`  
   若 wire 未开：`wire_dir=None`，文案注明 disabled。
2. `src/runtime/exec.rs:905` `InternalEvent::Error` 旁同样打 wire 字段（LlmHttp 路径）。
3. **可选（最小）：** `on_broken_stream` GiveUp 分支前一行 `tracing::warn!(…, "broken stream → GiveUp")` — **不改返回值**。默认做（一行），方便对照 agent Error。
4. **不**扩展 `TurnError` struct 进 wire 路径到协议/FE（避免协议面）；仅日志 + diagnostics `last-turn.json` 的 `last_error` / `wire_rel`。

**步骤 O2-3 · 失败保留策略**

1. **实现默认：** 不改 `KEEP_REQUESTS`/`KEEP_SESSIONS`；TurnError 时确保 `note_wire_pointer` + 日志路径已足够。
2. 文档（artifacts）：说明现 retention；建议排障时立刻复制 `<wire>/<sid>/` 尾部；未来可选 `LITECODE_LLM_WIRE_FAIL_ONLY=1`（**本包不实现**，标 待确认/后续）。
3. 写入 `WP-183/183-a/failure-retention.md`（短文）。

**步骤 O2-4 · Luna + grep runbook**

写入 `.litecode/ops/artifacts/WP-183/runbook-luna-grep.md`（父目录，brief 要求）+ 副本或链接说明于 `183-a/`。内容覆盖 WP-180 §8：

1. 开 `LITECODE_LLM_WIRE`（`serve.sh --wire`）+ `RUST_LOG` 含 `litecode.tool_audit`
2. 复现：文本 → ~2 tool → grep
3. 若整页刷新：收集 (a) wire 目录 (b) 进程是否仍在 (c) transcript 尾 (d) `diagnostics/<sid>/` crash + tool-audit 尾 + last-turn.json (e) 浏览器 console
4. 按 A/B/C 分类后再开 Fix 包 — **禁止未分类改 codec**

---

## 4. Diagnostics 目录约定（concrete）

```text
<data_root>/                          # = <workspace>/.litecode/
  sessions.db
  logs/litecode.log
  blobs/
  media/
  diagnostics/
    <session_id>/                     # sanitize 同 wire session_dir_name
      last-turn.json                  # 覆写
      tool-audit.jsonl                # append-only
      crash-<unix>.log                # panic hook；可多个
      wire-note.txt                   # 纯文本指针，非 symlink
    _process/                         # 无 TLS session 时的进程级 crash
      crash-<unix>.log
```

### `last-turn.json` schema（草案）

```json
{
  "session_id": "…",
  "turn_id": "…",
  "step": 0,
  "phase": "executing_tools",
  "last_tool": "grep",
  "last_call_id": "…",
  "last_error": null,
  "wire_rel": "wire/<session_id>/",
  "updated_unix_ms": 0
}
```

`phase` 字符串与 `TurnPhase` debug/snake 对齐即可（不必完美枚举）；未知写 `"unknown"`。

### `wire-note.txt` 例

```text
LITECODE_LLM_WIRE=/path/to/wire
session_folder=<sanitized>
absolute=/path/to/wire/<sanitized>
# copy summary.jsonl + latest *.response.sse.jsonl when filing a bug
```

---

## 5. 工具审计字段 schema

每行一条 JSON（jsonl）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `ts_unix_ms` | u64 | 必填 |
| `phase` | string | `start` \| `auth_ask` \| `auth_done` \| `exec_done` \| `panic` \| `cancel` |
| `session_id` | string | 必填 |
| `turn_id` | string? | TLS/参数可得则填 |
| `step` | u64? | |
| `tool` | string | |
| `call_id` | string | |
| `rule_id` | string? | auth 路径可得则填；否则省略 |
| `ok` | bool? | `exec_done` / `panic` / `cancel` 结束态 |
| `error_kind` | string? | 短类名：`panic` / `denied` / `timeout` / `canceled` / `output` / `other` |
| `error` | string? | 截断至 ≤200 chars |
| `auth_outcome` | string? | 仅 `auth_done`：`proceed` \| `deny` \| `abort` |

**目标问题：** 「某次 grep 是否 start / 是否卡在 auth / 是否已 exec_done」→ `rg '"tool":"grep"' tool-audit.jsonl` 即可。

---

## 6. 测试 / 验收

**重型规则：** `LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`；`CARGO_BUILD_JOBS=1` 或 2；**与 WP-181 互斥**；开跑前向 **backend_mid** 报槽。本 fill 阶段 **不跑 cargo**。

### 建议单测（实现时加；filter 示意）

| 测试 | 断言 |
|------|------|
| `runtime::diagnostics::tests::session_diag_dir_sanitizes` | `a/b` → `a_b`；空 → `_unknown` |
| `runtime::diagnostics::tests::append_tool_audit_roundtrip` | 写两行 jsonl，读回 phase/tool |
| `runtime::diagnostics::tests::write_last_turn_overwrites` | 两次写，文件为第二次 |
| `runtime::diagnostics::tests::panic_hook_writes_crash_file` | 子线程/`catch_unwind` 触发 hook 回调可测的 writer（或测 `write_crash_log` 纯函数），断言文件前缀 `crash-` 且含 payload 子串 |
| `tool::executor` 既有 tests 旁：mock Tool panic → 审计含 `phase=panic`（若引入 test-only audit sink） | 待确认：默认可先测 diagnostics 纯函数，executor 埋点用「源码扫描 / 手工」降低耦合 |

### cargo filter（实现阶段报槽后）

```bash
# 示意；实现后按实际模块名调整
LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=1 \
  cargo test -p litecode --lib diagnostics:: -- --nocapture
```

若 crate 名/二进制布局不同，以 `Cargo.toml` `[package] name = "litecode"` 为准（已核实）。

### 手工验收（Obs-2）

1. `serve.sh --wire /tmp/lc-wire` + 开 tool_audit 日志。
2. 人为 `panic!` 于测试工具或临时 feature（**默认不合并 panic 工具**；单测足够则跳过）→ 见 `diagnostics/…/crash-*.log`。
3. 正常 grep 一轮 → `tool-audit.jsonl` 含 start→auth_*→exec_done。
4. 合成/已有路径触发 TurnError（不修 codec）→ 日志含 wire 路径或 `wire disabled`。
5. runbook 文件存在且 checklist 可勾。

验收记录写入 `.litecode/ops/artifacts/WP-183/183-a/acceptance.md`（实现后）。

---

## 7. 产物路径

| 路径 | 内容 |
|------|------|
| `.litecode/ops/artifacts/WP-183/183-a/plan-draft.md` | 本文件（fill） |
| `.litecode/ops/artifacts/WP-183/183-a/` | 实现 diff 说明、acceptance、failure-retention.md |
| `.litecode/ops/artifacts/WP-183/runbook-luna-grep.md` | Luna+grep 排障 runbook（父 WP-183 目录） |
| `.litecode/ops/artifacts/WP-183/183-a-brief.md` | 已有 brief |

---

## 8. 与 WP-181 文件冲突说明（只读 / 不改）

| 文件 / 区域 | 本包 | WP-181（186/187） |
|-------------|------|-------------------|
| `src/runtime/phase.rs` `PhasePermissionSink` | **不改** | 181-b 可能改 phase 投影 |
| `src/permission/**`、AskOutcome、reject_all | **不改** | 181-a |
| `src/tool/authorize.rs` 控制流 | **不改**（审计在 `run_tool` 外包一层） | 若 181 需小改 authorize，互斥合并 |
| `src/session/**` writer / seal / snapshot | **不改**；仅读 `data_root` 路径写 diagnostics 文件 | 181-a 主战场 |
| `src/client_protocol/**` waiting_for_user / pending_permissions | **不改** | 181-a |
| `src/runtime/mod.rs` `run_with_turn` / `emit_turn_completed` | 可加 TLS enter + Error 日志（observe） | 若 181-b 同时改，**串行/报槽**；本包 diff 限于诊断调用 |
| `src/tool/executor.rs` / `pipeline.rs` | 本包主改（审计） | 181 预期不碰 |
| `src/llm/codec/wire_dump.rs` | 可选只读 helper；**不改 retention 常数** | 不碰 |
| `src/cli/cli.rs` | 装 hook（一行级） | 不碰 |

**结论：** 与 WP-181 **无必然写冲突**；唯一重叠风险是 `runtime/mod.rs` 同文件编辑 —— 实现前与 backend_mid 确认 186/187 进度，冲突则本包先只加 `diagnostics` 模块 + tool 审计，runtime 收口日志后合。

---

## 9. done_when checklist（镜像父 plan §6）

- [ ] panic / diagnostics 路径约定落地且可演示（hook 或等价写入 + 目录说明）
- [ ] 工具审计能回答「某次 grep 是否 start/auth/exec」
- [ ] TurnError/GiveUp 日志能指到 wire 路径（在 wire 开启时）
- [ ] Luna+grep runbook 写入 `.litecode/ops/artifacts/WP-183/`
- [ ] 相关测或手工验收记录；**未**夹带 H1 codec 大修；**未**改权限桥
- [ ] 与 WP-181 无文件冲突说明（§8；若有重叠注明只读/未改）

### 本 fill 额外

- [x] 本 `plan-draft.md` 完整，backend_mid 可批而不必再问澄清（未知已标默认）
- [ ] submit 后等 approve → 实现；重型报槽

---

## 10. 待确认与默认选择（汇总）

| 项 | 默认 |
|----|------|
| diagnostics 模块路径 | `src/runtime/diagnostics.rs` |
| panic hook 安装时机 | `cli::run` 在 `setup_logging` 之后 |
| wire 指针 | `wire-note.txt` 文本，不用 symlink |
| 审计埋点位置 | `run_tool` 外包 authorize，**不改** `authorize.rs` 分支 |
| wire 路径 helper | diagnostics/日志侧读 env，尽量不改 `wire_dump` API |
| 失败保留 | 本包只文档化；不实现 `FAIL_ONLY` |
| `TurnError` struct | 不扩协议字段；只日志 + last-turn.json |
| runtime/mod.rs 与 181-b | 报槽串行；冲突则分期合入 |

---

## 11. 实现时预计 touch 文件清单

**新建**

- `src/runtime/diagnostics.rs`（+ `mod diagnostics` in `src/runtime/mod.rs`）
- `.litecode/ops/artifacts/WP-183/runbook-luna-grep.md`
- `.litecode/ops/artifacts/WP-183/183-a/failure-retention.md`
- `.litecode/ops/artifacts/WP-183/183-a/acceptance.md`（实现后）

**修改（observe）**

- `src/cli/cli.rs` — install hook
- `src/tool/executor.rs` — audit phases
- `src/tool/pipeline.rs` — join panic audit
- `src/runtime/mod.rs` — DiagContext guard + Error 路径 wire 日志
- `src/runtime/exec.rs` — Error emit 旁 wire 日志（可选同行）
- `src/llm/codec/http.rs` — GiveUp 前一行 warn（可选、默认做）

**明确不改**

- H1 / responses·chat codec 解析逻辑
- `PhasePermissionSink`、AskOutcome、session seal、FE
