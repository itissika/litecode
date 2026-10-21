# WP-200 dig-survey — run_code 模型默认视图摸底

> **作者：** Runtime survey（执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD：** `feat/tool-contract-ptc` @ `c688b0a`  
> **范围：** 只读；未改产品代码、未跑 cargo、未 commit  
> **依据：** `brief.md`、`plan.md`；源码 `src/tools/code.rs` + `src/tools/code/render.rs` + `src/ptc/*` + 测例 / SDK

---

## 0. 一句话现状

`run_code` 经 `CodeTool::execute_code` → `ptc::execute` → `CallLedger` 聚合 → `run_report` → **`render_model_view`**，把一整次脚本跑成**单段纯文本**塞进 `ToolCallResult.content`。默认 agent 面是**观测台**：header + 独立 failure + error_details + diagnostics + **全量/优先 ledger** +（自愿）logs + result。Brief 目标三块（Head / Logs? / Result?）尚未落地；成功也不静默；失败除 `Cancelled` 外**抹掉** `value`。

---

## 1. 入口与聚合结构

### 1.1 入口

| 项 | 位置 |
|---|---|
| 绑定名 | `CODE_TOOL_NAME = "run_code"`（`src/tools/code.rs`） |
| 工具类型 | `CodeTool` / `CodeToolHost` |
| 执行 | `CodeTool::execute_code` → `ptc::execute(ExecuteRequest{…})` → `render_run_output` |
| 入参 schema | `{ code: string, timeout_ms?: u64 }` |
| 出参 | `ToolCallResult`：成功 `ok(content)`；失败 `error_with_kind(kind, content, retryable)`（content = 模型面文本） |
| description | `CodeTool::description` ← `code_manifest` 生成的 SDK 长文（`self.sdk`） |

### 1.2 运行时结果（内部，ptc）

`src/ptc/mod.rs`：

```text
Outcome {
  logs: Vec<LogEntry>,          // level + message（脚本 console 等）
  value: Option<Json>,          // 脚本 return
  failure: Option<Failure>,     // kind/message/stack/location/tool_error/…
  calls: Vec<CallRecord>,       // seq, name, started, ended?, state
}
Failure { kind: FailureKind, message, stack?, location?, at_end_of_input, tool_error?, … }
CallRecord { seq, namespace, name, started: Duration, ended: Option<Duration>, state: CallState }
```

**注意：** `Outcome` **没有**顶层 `elapsed`/`duration` 字段；wall 时钟在 `BudgetClock::wall_elapsed`（`src/ptc/budget.rs`），仅用于各 call 的 `started`/`ended`，**未**汇入模型 head。

### 1.3 Host 账本（内部 → 喂渲染）

`src/tools/code.rs`：

| 类型 | 关键字段 |
|---|---|
| `LedgerLine` | `seq, tool, args_summary, outcome?, error_code?, error_message?, diagnostics, session_read, priority` |
| `LedgerSnapshot` | `lines, omitted, omitted_diagnostics, tool_counts, error_counts, session_read_hidden` |
| `CallLedger` | 有界保留 `LEDGER_KEEP=256` + `PRIORITY_CAP=64`（error/reject/unsettled/partial edit） |

嵌套工具经 `execute_tool_call`；`CallLedger::record` 抽 `call_outcome_label` / `call_error_code` / `call_error_message` / `bounded_diagnostics`。

### 1.4 模型面报告（渲染输入）

`src/tools/code/render.rs` → `RunReport`：

| 字段族 | 字段 |
|---|---|
| logs / result | `logs: Vec<(level,msg)>`, `value: Option<Value>` |
| failure | `failure_text`, `failure_kind`, `failure_location`, `failure_at_end`, `failure_stack`, `tool_error`, `uncaught_type_error`, `resource`, `timeout_ms`, `output_limited` |
| ledger | `calls: Vec<LedgerSnap>`, `calls_omitted`, `tool_counts`, `error_counts`, `call_total`, `unsettled_total`, `session_read_hidden` |
| diagnostics | `diagnostics`, `diagnostics_omitted` |
| source | `source`（失败定位用） |

`LedgerSnap`：`seq, tool, summary, state, error_code?, error_message?, session_read`  
`UncaughtToolError`：`seq, tool, code, message, summary, details?`

### 1.5 预算常量（默认面）

| 常量 | 值 | 用途 |
|---|---|---|
| `RUN_CODE_VIEW_MAX_BYTES` | 65_536 | 整段模型面硬顶 |
| `HEADER_CAP` | 1KiB | header 块 |
| `FAILURE_CAP` | 4KiB | 独立 failure 段 |
| `ERRORS_CAP` | 3KiB | error_details 段 |
| `DIAGNOSTICS_CAP` | 4KiB | diagnostics 段 |
| `LEDGER_CAP` | 8KiB | ledger 段 |
| `LOGS_FLOOR` | 12KiB | logs 保底预算 |
| `DIAGNOSTIC_CAP` / `DIAGNOSTIC_RENDER_CAP` | 16 | 聚合/渲染条数 |
| `DIAGNOSTIC_CODES` | `lsp_error`, `edit_block_failed`, `path_outside_workspace`, `sensitive_path` | 进模型面的诊断码 |

---

## 2. 默认渲染 / 序列化路径

### 2.1 组装顺序（`render_model_view`）

```text
render_fixed:
  1. render_header_block     // 永远有
  2. render_failure?         // ToolError 或 failure_text/location/stack
  3. render_error_details?   // uncaught + permission_denied 等复述行
  4. render_diagnostics?     // "diagnostics:" 列表
  5. render_ledger?          // "calls (N of M shown):" + 逐行
然后按窗拼：
  6. render_logs?            // "logs:"；空则整段省略
  7. render_result?          // "result:\n…"；无 value 则省略
```

段之间 `\n\n`。**无 JSON 副本合同**（已删 `@@run_code_data`；测例负向断言仍在）。

### 2.2 哪些默认进 agent 视图

| 层 | 成功路径 | 失败路径 | brief 目标 |
|---|---|---|---|
| **Header** | `run_code ok · N calls (tool×k) · R rejected …` | `run_code failed: … · finished/total, unsettled`；或 uncaught ToolError 头 | **保留并收敛**为 Head 不变量 |
| **failure 大段** | 通常无 | 有：正文 / line+col+源码行+caret / stack（Exception、Stack OOM） | **硬清**（错因收进 Head） |
| **error_details** | 可有（如 permission_denied 行） | 常有；与 ToolError `code: message` 重复 | **硬清** |
| **diagnostics:** | 可有（如 `edit_block_failed`、`lsp_error`） | 可有 | **硬清**（默认同层） |
| **ledger `calls (`** | **有调用就出**（含全部 ok 行，优先保留 error/partial） | 有 | **硬清**（默认面） |
| **logs:** | 仅脚本主动写才出 | 同；timeout 测保留 tail | **保留（自愿）** |
| **result:** | 有 return 则出 | **当前多数失败不出**（见 §3） | **保留；失败尽量保** |

### 2.3 内部 / 非默认面

- `CallLedger` / `LedgerSnapshot` / `Outcome.calls`：执行期结构；可给人看/测，但 brief 允许**内部通道**，禁止默认 `render_model_view` 再铺。
- `ToolCallResult.diagnostics`（嵌套工具）：进账本后由 `aggregate_diagnostics` 滤四种 code → 模型面 diagnostics 段（待砍）。
- `transcript_file::ledger_reads_sessions`：靠模型面文本含 `· reads session store`（ledger 省略行）探测 session-read；**依赖默认 ledger 文案**（改面时必须迁 Head 或内部通道）。
- UI：历史 remediation 文档称消息列表 raw 显示 `run_code` content；本 survey 未跟前端。

### 2.4 Header 现状句式（对照 brief「成功数/失败数/耗时」）

- 成功：`run_code ok · {total} calls (grep×2, read×1) · {n} rejected (not_found×n), handled by the program`
- 失败：`run_code failed: timeout after {timeout_ms}ms · {finished}/{total} calls finished, {unsettled} unsettled`
- 另可附 unsettled 旁注段（副作用提醒）
- **无耗时（elapsed）**；**无显式「成功数 / 失败数」**二元计数（有 total、tool×、rejected×、finished/unsettled）

---

## 3. 成功静默 vs 失败可见；partial

| 议题 | 现状 | brief |
|---|---|---|
| 成功静默 | **否**。只要有嵌套调用就渲染 **ledger**（ok 行也列）；有 diagnostics / rejected 时 header 与 diagnostics/error_details 也吵 | 成功静默：无 failure/diagnostics/ledger/error_details |
| 失败可见 | **是，且过厚**：header + failure 大段（含 stack）+ error_details + diagnostics + ledger | 失败可见但收进 Head（+自愿 logs/result），不另堆观测层 |
| 失败保 partial result | **否（关键缺口）**。`render_run_output`：`keep_value = value.is_some() && failure.is_none_or(Cancelled)`；注释 *「A failed program does not present its value」* → timeout/exception/syntax/ToolError/OutputLimit 等 **一律不呈现 result** | 失败尽量保已有 result/partial（学 pi） |
| logs 在失败时 | 仍可渲染（timeout 测断言保留 `LOG_TAIL_SENTINEL`） | 保留 |

Cancelled 且已有 return：可同时带 value + Cancelled failure（ptc/engine 约定）；这是少数「失败仍见 result」路径。

---

## 4. 与工具 `{ok,message}` / ToolError 重复复述

| 位置 | 复述内容 |
|---|---|
| `render_failure`（uncaught ToolError） | `{code}: {message}` + details JSON 前缀（≤300）——与工具/ToolError 同义 |
| `render_error_details` / `format_error_detail` | `#seq tool summary: code: message`——再抄一遍 error_message |
| Header uncaught 行 | `uncaught ToolError {code} from call #…` + rejected 子句 |
| Ledger `state` | `error:{code}` / `ok(a/b)`；行本身不带 message，但 error_details 补 message |
| `CallLedger::record` ← `call_error_message` | 从 `ToolCallResult.error.message` 截 300 字进 snap，专供上述复述 |
| SDK/教法 | PRELUDE：`EditResult{ok,path,message}` 归工具；同时又说 LSP「appear only in this run's **outer text**」——把诊断导向外层观测面 |

嵌套成功路径的 `data`（含 `{ok,message}`）**默认不**整包进主视图（只进脚本作用域）；问题在外层又用 ledger/error_details/failure **二次叙述**契约错误。

---

## 5. 测例路径与旧分层名

### 5.1 主测（断言默认面分层）

| 文件 | 覆盖的旧分层 / 符号 |
|---|---|
| `tests/run_code_model_view.rs` | `run_code ok`、`result:`、`@@run_code_data` 负向、`timeout after`、`line N, column`、`program.js:`、`calls (` ledger、`of 466`、`read×466`、`· reads session store`、`ledger_reads_sessions`、`ok(9/10)`、`edit_block_failed`、`N rejected (…)`、`uncaught ToolError`、`error:invalid_input` |
| `src/tools/code/render.rs` `mod tests` | `result:` 窗、`diagnostics:` 条数帽、`rejected`/`×`、header 完整/省略 ledger、`calls (` / omission、failure header + unsettled 旁注、`[output limit reached]`、ledger 行 vs `#seq … code: message` |
| `src/tools/code.rs` `mod tests` | unsettled/cancelled merge、permission_denied ledger 行、`@@run_code_data` 负向、priority ledger、session-read 省略 + `ledger_reads_sessions`、ledger ≤8KiB |
| `tests/code_surface_phase3.rs` 等 | ledger 含 `error:permission_denied`、`N rejected (permission_denied×1)` |
| `tests/code_surface_phase4.rs` | ledger 行尾 `error:permission_denied` / `error:invalid_input` |
| `tests/code_tool_runtime.rs` | 真循环行为（非专门分层，但依赖 content 文本） |
| `src/session/transcript_file.rs` 测 | `ledger_reads_sessions` / `RUN_CODE_SESSION_NOTE` |

### 5.2 硬清时测例必改断言关键词（便于表）

`calls (` · `diagnostics:` · `edit_block_failed`（作为**外层段**）· `N rejected (` · `#1 ` error_details · `ledger_reads_sessions` 依赖模型面 · failure stack/`program.js:` 大段 · `ok(9/10)` **仅**出现在 ledger 行时

（Header 内简短失败标签 / 成功失败计数 **不是**禁止，而是 brief 要的 Head 不变量。）

---

## 6. Tools 教法 / SDK / description 观测字段

| 路径 | 与 run_code 观测相关的句子 / 字段 |
|---|---|
| `src/tool/code_manifest.rs` `PRELUDE` | ToolError / TypeError；「LSP errors … **outer text**」；「model sees about **64KB**」；`undefined`→null；`EditResult{ok,path,message}` |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | PRELUDE 快照；字段注释含「**outer ledger** or transcript」（经 coords → edit schema） |
| `src/tool/coords.rs` | `OLD_STRING_HINT` 类：「line-number prefixes pasted from **outer ledger**…」 |
| `src/tools/descriptions/write.md` | LSP → outer text（与 PRELUDE 同） |
| `src/tools/descriptions/{edit,apply_edits,grep,glob,read,run,wait,kill}.md` | 教嵌套工具契约；**无**独立教 failure/ledger/diagnostics **段**；多处「inside `run_code`」 |
| `src/tools/descriptions/lsp.md` | 产品动作名 `diagnostics`（≠ 模型面 `diagnostics:` 段） |
| WP-200 已有草稿 | `dig-tools-teaching.md`、`draft-run_code-view.md`（artifacts only，live 未动） |

`CodeTool` **没有**单独的 `descriptions/run_code.md`；教法 = PRELUDE + 各绑定 md 拼进 SDK。

---

## 7. pi / codemode 对标

| 来源 | 内容 |
|---|---|
| `brief.md` / `plan.md` | 明确「学 **pi** codemode：失败保 partial、少噪音、硬清不双轨」 |
| `src/tools/code*` / `src/ptc*` | **无**指向外部 pi 实现的注释或链接；`ptc` = 本仓 embedded JS runtime（「code mode」产品名） |
| `docs/ptc/*` | 产品 charter / remediation（历史观测台设计）；非 pi 源码对标 |
| 其它 artifacts | 偶发「pi-view」等旧词，与本 WP 无关 |

**结论：** pi 是**设计口径**，不是可 diff 的 vendored 参考；行为对齐以 brief 三条（失败保产出、嵌套结果不进主视图、勿堆方言）为准。

---

## 8. 建议硬清清单（符号 / 字段 / 渲染路径）

**默认 `render_model_view` 删除或不再调用：**

1. `render_failure` + `FAILURE_CAP` + failure 正文/stack/源码 caret 大段（定位信息**折叠进 Head** 一两行）
2. `render_diagnostics` + `diagnostics:` 段 + `DIAGNOSTICS_CAP` / `DIAGNOSTIC_RENDER_CAP`（默认面）
3. `render_ledger` + `calls (N of M shown)` + `LEDGER_CAP` + `format_ledger_line` / omission 行（默认面）
4. `render_error_details` + `ERRORS_CAP` + `format_error_detail`
5. Header 过长 rejected 子句若与「成功数/失败数」重复，收敛为 brief 计数，勿双轨叙述
6. `keep_value` 失败抹 value 逻辑 → 改为失败仍 `present_returned_value`（窗内）
7. 教法：`outer text` / `outer ledger` 导向（Tools 收口；见已有 dig-tools-teaching）

**可留内部、不进默认面（Q4）：** `CallLedger` 结构、`aggregate_diagnostics`、测专用断言通道、`ledger_reads_sessions` 的**非模型面**替代信号。

**测例：** §5 所列凡钉死 `calls (` / `diagnostics:` / 独立 failure 段 / 靠 ledger 文本做 session 探测的，一律改写。

---

## 9. 建议保留的不变量（Head / Logs / Result）

| 块 | 保留什么 | 现状缺口 |
|---|---|---|
| **Head** | 跑不起来：行(+列)+错因；跑起来：成功数/失败数/**耗时**；过程失败可带短标签；可选 unsettled 旁注（宜仍在 Head 内） | 有 ok/failed 头与 rejected/finished 信息，但**无 elapsed**；成功仍夹 ledger |
| **Logs** | 脚本主动日志；空则整段无；`logs:` 窗与 output-limit 标记策略可沿用 | 已较接近 brief |
| **Result** | `result:` + `RUN_CODE_VIEW_MAX_BYTES` 头尾窗 / UTF-8 安全切断 | **失败空抹**必须改；成功路径已按窗保 |

窗预算：砍掉 fixed 观测段后，logs/result 可自然吃到更多字节（`remaining_after` / `logs_budget` 逻辑可简化但行为目标不变）。

---

## 10. 主要文件路径

| 角色 | 路径 |
|---|---|
| 入口 / 账本 / `render_run_output` | `src/tools/code.rs` |
| 模型面渲染 | `src/tools/code/render.rs` |
| JS runtime / Outcome | `src/ptc/{mod,engine,budget,boundary,calls,output}.rs` |
| SDK PRELUDE | `src/tool/code_manifest.rs` |
| SDK fixture | `src/tool/fixtures/run_code_sdk.{txt,sha256}` |
| Diagnostic 类型 | `src/tool/outcome.rs` |
| ToolCallResult | `src/types/result.rs` |
| session ledger 探测 | `src/session/transcript_file.rs` |
| coords「outer ledger」 | `src/tool/coords.rs` |
| 集成测 | `tests/run_code_model_view.rs`, `tests/code_tool_runtime.rs`, `tests/code_surface_phase{1,2b,3,4,5}.rs` |
| 历史设计 | `docs/ptc/run-code-remediation-plan.md`, `docs/ptc/charter.md` |
| WP-200 已有 | `artifacts/WP-200/{brief,plan,dig-tools-teaching,draft-run_code-view}.md` |

---

## 11. 需 mid → GM 拍板的歧义（最多 5）

| # | 议题 | 调查结论 / 建议默认 |
|---|---|---|
| **Q1** | Head「成功数/失败数」计数口径：嵌套 call 的 ok vs `error:*`？是否计入 unsettled？caught ToolError（program handled）算失败还是「rejected handled」？ | 建议：成功 = `ok`/`ok(a/b)` 且非 error；失败 = `error:*`；unsettled **单列**或并入失败旁注，勿 silently 算成功。与现 `error_counts`/`tool_counts` 可对齐但句式改 brief。 |
| **Q2** | **耗时**数据源：`Outcome` 无 elapsed；用 `BudgetClock` 墙钟、active budget 已用、还是 `max(call.ended)`？单位 ms？ | 建议：墙钟整次 run（pause 审批是否计入需一句定）；渲染 `… · {ms}ms`。需 ptc 把 elapsed 挂上 Outcome 或 render 入参。 |
| **Q3** | 失败时 Head 是否仍展示 **line+col + 单行源码**（现 failure 段能力），还是只「行号+错因」纯文本、**永不** stack？ | brief：精准定位、不堆栈。建议：syntax/exception → Head 内 line/col+一行 snippet；stack **默认丢**（或仅内部测）。 |
| **Q4** | `ledger_reads_sessions` / session-read 产品信号：迁 **Head 一句**、改内部 API，还是产品不再需要？ | 建议：若仍要 → Head 固定短句；**禁止**再依赖 `calls (` 省略行。Tools/transcript 测同步。 |
| **Q5** | 砍默认 diagnostics 后，`lsp_error` / `edit_block_failed`：只留在工具 data / 脚本自 log，还是 Head 一行摘要？ | 建议：不进默认面；PRELUDE 改「不进 resolve 则自行 log/return」。与 dig-tools-teaching D 一致。 |

（plan.md 已有 Q1–Q5 方向性默认「是」；上表是**实现口径**歧义，供 fill 前拍板。）

---

## 12. 非目标 / 本 dig 未做

- 未改代码、未跑 cargo、未 commit、未合 main  
- 未改 live description / SDK fixture  
- 未深挖前端 TOOL_VIEWS（remediation 文档称已 raw 显示）  
- 未实现硬清（本文件仅摸底）
