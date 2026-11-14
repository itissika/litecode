# LiteCode run_code / 工具接口观测与评测基础设施改造报告

日期：2026-11-14

状态：评测数据复盘完成；Phase 1 / Phase 2 已实现，等待下一次正式评测验证。

范围：LiteCode `run_code` 的嵌套工具调用、输入字段、输出字段读取、上浮提醒，以及评测后数据自动落盘。

## 1. 结论摘要

这组评测数据支持三个判断：

1. **绑定面太厚**：本次 4 个任务绑定了 13 个 code tools，实际只调用了 5 个；`run/wait/kill`、`edit`、`glob`、`grep`、`webfetch`、`websearch` 都是 0 调用。
2. **结果面太厚**：agent 在 JS 里真正读取的输出字段很少，主要是 `exec_command.output`、`read.lines`、`exec_command.exit_code`、`exec_command.session_id`；`path_risk`、`diagnostics`、`clamped`、`has_more`、`total_lines`、`wall_time_seconds`、`original_token_count`、`truncated` 等字段没有在程序逻辑中被读取，主要由 renderer 生成提醒，或在整对象 `console.log` 时被动带入 context。
3. **提醒本身不贵，但提醒事实的来源放错了层**：本次 14 条上浮提醒合计 349 cl100k tokens，占 43 次 `run_code` 输出总 token 的 1.6%；而为 `PathRisk` / `ToolDiagnostic` / `ClampedParam` 及可选字段准备的公开 SDK 面约 425 tokens/请求。真正的问题是：宿主已经知道的事实没有进入提醒，或者公开结果字段和提醒重复表达同一事实。

下一阶段方向：

- **评测基础设施**：在 tool 层把每次调用的结构化事实自然落盘，评测后由 collector 自动汇总；不动 `run_code` 实现。
- **工具面**：评测 profile 不减工具，grep/glob/web 全部保留；只把 `edit` 合并进 `apply_edits`，删除死字段，再把 agent 结果字段和提醒数据分开。
- **提醒**：由系统生成，必须可执行、可去重，并带上关键事实（特别是 `session_id` 和 clamp 的 applied 值）。

## 2. 数据口径与限制

### 2.1 数据来源

本报告使用 `selected.json` 指向的 4 个 LiteCode trial：

| 任务 | trial |
|---|---|
| `cancel-async-tasks` | `cancel-async-tasks__3NacX6s` |
| `password-recovery` | `password-recovery__MsAQXRE` |
| `polyglot-c-py` | `polyglot-c-py__N5dvbmq` |
| `prove-plus-comm` | `prove-plus-comm__FunYw2q` |

试验详情：

- binary commit：`e5013136e3cd`，pre-final-cutover
- 每任务 1 trial
- 47 次 LLM 请求
- prompt 905,261
- completion 58,400
- total 963,661
- cost `$0.042522846`
- 43 次外层 `run_code` 调用
- 76 次实际嵌套工具调用
- 61 个源码级嵌套调用点（部分调用点在循环里）

### 2.2 注意：报告文件与 selected.json 不一致

`eval/bench5-report.json` 当前仍是旧的 63 requests / 1,449,443 tokens 汇总；`selected.json` 指向的 cterm1 结果是 47 requests / 963,661 tokens。本文使用后者及对应的原始 `sessions.db` / `litecode.log`，不使用旧汇总。

后续 collector 需要把 selected 口径和报告生成口径统一，否则报告数字会继续漂移。

### 2.3 统计方法

- 外层调用：`sessions.db` 的 `transcript_items.item_type = function_call`，`name = run_code`。
- 嵌套调用：`litecode.log` 的 `tool_audit`，`call_id` 含 `:code:`，格式为 `parent_call_id:code:N`。
- 输入字段：对 `run_code` 的 `arguments.code` 做 TypeScript AST 解析，统计 `tools.<suite>.<member>` 调用点的一级对象键。
- 输出字段：对结果变量做 AST 属性读取统计；同时单独统计整对象 `console.log` / `JSON.stringify`，因为整对象会把所有字段带入模型上下文。
- 提醒：`function_call_output.output` 中以 `↳` 开头的行。
- SDK 面 token：按 `sdk/generated/sdk-fragments.json` 用当前 `code_sdk` 规则本地重放，并以 cl100k_base 计数。实际请求 header 里的 `tools_tokens` 是权威观测值。

### 2.4 限制

- 只有 4 个任务、每任务 1 trial；频率是描述性的，不能当作全局结论。
- 4 个任务偏 shell / 数据 / 证明，不能用来判断 LSP、browser、memory 是否该删；这些工具在 eval profile 里本来就是 off。
- 本次是 pre-cutover 面，仍绑定 `run/wait/kill`；post-cutover 面只有静态估算，没有新的行为数据。
- AST 字段读取是静态分析，不覆盖动态 `r[field]`、spread 等极端写法；整对象打印单独统计。

## 3. 现状

### 3.1 外层 API

模型实际只看到 2 个外层工具：

| 外层工具 | 调用次数 |
|---|---:|
| `run_code` | 43 |
| `todo` | 0 |

`todo` 的 schema 约 245 cl100k tokens，47 次请求都带着它，但 0 调用。

每个 `run_code` 的嵌套调用数：

| 嵌套调用数 | 次数 |
|---:|---:|
| 1 | 26 |
| 2 | 10 |
| 3 | 2 |
| 5 | 1 |
| 19 | 1 |
| 0 | 3 |

40/43 次至少调用一个工具，平均 1.77 个嵌套调用。

### 3.2 嵌套工具调用频次

| 工具 | 实际调用 | 占比 | 源码调用点 |
|---|---:|---:|---:|
| `exec_command` | 57 | 75.0% | 41 |
| `file.write` | 11 | 14.5% | 12 |
| `file.read` | 4 | 5.3% | 4 |
| `write_stdin` | 3 | 3.9% | 3 |
| `file.apply_edits` | 1 | 1.3% | 1 |
| `file.edit` | 0 | 0 | 0 |
| `file.glob` | 0 | 0 | 0 |
| `file.grep` | 0 | 0 | 0 |
| `web.fetch` | 0 | 0 | 0 |
| `web.search` | 0 | 0 | 0 |
| `process.run` | 0 | 0 | 0 |
| `process.wait` | 0 | 0 | 0 |
| `process.kill` | 0 | 0 | 0 |

分任务：

| 任务 | run_code | 嵌套总数 | exec | write | read | stdin | apply_edits |
|---|---:|---:|---:|---:|---:|---:|---:|
| cancel-async-tasks | 9 | 17 | 8 | 6 | 1 | 2 | 0 |
| password-recovery | 17 | 18 | 16 | 1 | 0 | 1 | 0 |
| polyglot-c-py | 9 | 31 | 26 | 2 | 2 | 0 | 1 |
| prove-plus-comm | 8 | 10 | 7 | 2 | 1 | 0 | 0 |

`exec_command` 是绝对主力。4 个任务里的搜索、列目录、找文件基本都在 shell 里完成（`find` / `grep` / `ls` / `od` / `xxd` / `strings`），所以 `file.grep` / `file.glob` 是 0 调用。

### 3.3 输入字段频率

源码调用点统计：

| 工具 | 输入字段 | 次数 |
|---|---|---:|
| `exec_command` | `cmd` | 41/41 |
| `exec_command` | `yield_time_ms` | 41/41 |
| `exec_command` | `workdir` | 10/41 |
| `exec_command` | `max_output_tokens` | 10/41 |
| `exec_command` | `tty` | 1/41 |
| `file.write` | `path` | 12/12 |
| `file.write` | `content` | 12/12 |
| `file.write` | `create_only` | 0/12 |
| `file.write` | `expected_hash` | 0/12 |
| `file.read` | `path` | 4/4 |
| `file.read` | `start_line` | 2/4 |
| `file.read` | `limit` | 2/4 |
| `write_stdin` | `session_id` | 3/3 |
| `write_stdin` | `chars` | 3/3 |
| `write_stdin` | `yield_time_ms` | 3/3 |
| `write_stdin` | `max_output_tokens` | 1/3 |
| `apply_edits` | `edits` | 1/1 |
| `run_code` | `code` | 43/43 |
| `run_code` | `timeout_ms` | 0/43 |

结论：

- `exec_command` 的输入面不厚：`cmd` + `yield_time_ms` 是绝对主力。
- `write` 的 `create_only` / `expected_hash` 在本次样本里完全没用。
- `run_code.timeout_ms` 43 次全部没用。
- `tty` 只用 1 次，但它是 PTY/Ctrl-C 场景的关键输入，不能简单删。

### 3.4 输出字段读取

直接属性读取（源码级）：

| 读取 | 次数 |
|---|---:|
| `exec_command.output` | 31 |
| `exec_command.exit_code` | 4 |
| `exec_command.session_id` | 4 |
| `exec_command.stderr` | 3（字段不存在，触发 3 次契约提醒） |
| `file.read.lines` | 4 |
| `file.write.ok` | 1 |
| `file.write.file` | 1 |
| 其他所有字段 | 0 |

整对象暴露：

- `exec_command`：9 次 `console.log(r)` + 1 次 `JSON.stringify(start)`，共 10/41 个调用点把整个结果对象打印出来。
- `file.write`：6 次整对象打印，4 次 bare await（结果直接丢掉），1 次只读 `ok/file`，1 次完全未用。
- `write_stdin`：3/3 全部整对象打印。
- `apply_edits`：1/1 整对象打印。
- `file.read`：4/4 只读 `lines`，分页字段 0 直接读取。

实际 agent 需要的最小结果大致是：

| 工具 | 实际需要 | 现在多给的 |
|---|---|---|
| `exec_command` | `output`, `exit_code?`, `session_id?` | `wall_time_seconds`, `original_token_count`, `truncated` |
| `write_stdin` | 同上 | 同上 |
| `write` | `created`, `file.path` | `file.hash`, `file.bytes`, `file.abs`, `path_risk`, `diagnostics` |
| `read` | `lines`, `path` | `start_line`, `total_lines`, `has_more`, `skipped_in_context`, `lines_capped`, `clamped` |
| `apply_edits` | `ok`, `path`, `reason`, `detail` | `nearest_line`, `path_risk`, `diagnostics` |

这些额外字段中，有些确实是宿主需要的（`has_more` / `total_lines` 生成分页提醒，`path_risk` 生成路径提醒，`truncated` 生成截断提醒，`clamped` 生成参数提醒）。问题是：**宿主需要，不等于 agent 需要在 JS 里读到。**

### 3.5 固定 SDK 面成本

本地 cl100k 估算：

| 组成 | 当前 10 工具面 |
|---|---:|
| prose | 405 |
| declarations | 358 |
| `libs` / `fs` | 598 |
| types | 436 |
| tools interface | 2,165 |
| 合计 | ~3,973 |

本次 pre-cutover 的 13 工具面约 4,590 tokens。

单工具边际（doc + signature，近似）：

| 工具 | tokens |
|---|---:|
| `grep` | 544 |
| `apply_edits` | 268 |
| `read` | 249 |
| `write` | 240 |
| `glob` | 219 |
| `exec_command` | 135 |
| `websearch` | 137 |
| `webfetch` | 126 |
| `write_stdin` | 119 |
| `edit` | 91 |
本次实际只用了 5 个工具（`apply_edits`、`exec_command`、`read`、`write`、`write_stdin`）：

- 当前 10 工具面 3,973；5 工具面约 2,845；差额约 **1,128 tokens/请求**。
- pre-cutover 13 工具面 4,590；5 工具面约 2,866；差额约 **1,724 tokens/请求**。
- `libs` 598 tokens，43 次代码里 **0 次使用**。
- `fs` 153 tokens，5 次引用，集中在 3 次调用。
- `todo` 约 245 tokens，0 调用。
- 实际请求 `tools_tokens = 5,472`；47 次请求合计 257,184 prompt tokens，占 905,261 prompt 的 **28.4%**。这是上下文占用，不是账单占比；缓存命中会让实际费用小得多。

### 3.6 上浮提醒

43 次输出里，14 条 `↳` 提醒：

| 类型 | 次数 | 工具 | 例子 |
|---|---:|---|---|
| `path_risk:outside` | 5 | `file.write` | `this write lands outside the workspace: /tmp/...` |
| `contract:unknown_field` | 3 | `exec_command` 预检 | `result has no field 'stderr'. fields: ...` |
| `process:live_session` | 3 | `exec_command` 2 / `write_stdin` 1 | `session is still running; use write_stdin with its session_id` |
| `read:truncated_page` | 2 | `file.read` | `showing lines 1-20 of 143; continue with start_line 21` |
| `failure:swallowed` | 1 | `write_stdin` | `failed(not_found); the program kept going` |

总量：

- 14 条，349 cl100k tokens
- 平均 24.9 tokens/条，最大 36
- 11/43 次输出含提醒，占 25.6%
- 占输出总 token 的 **1.6%**，占字节的 3.1%

所以提醒文本本身不贵；贵的是为提醒准备的公开字段和类型。
重复：

- `stderr` 警告完全相同，出现 3 次。
- `/tmp/t_edge.py` 的 path risk 完全相同，出现 2 次。

### 3.7 两个真实缺陷

#### 缺陷 A：live session 提醒不带 `session_id`

3 次 live session 提醒的文本都是：

> session is still running; use `tools.process.write_stdin` with its session_id

但没有把 `session_id` 打出来。

`password-recovery` 第 58 次调用：

1. `exec_command` 请求 `yield_time_ms: 60000`，被静默 clamp 到 30000；
2. 返回 live session `bg_599ed2b3`；
3. 模型只打印了 `r.output`，输出为空，所以 context 里没有 id；
4. 下一轮它调了 `write_stdin({session_id: "1"})`；
5. 返回 `not_found`，又生成一条 swallowed failure 提醒。

宿主明明知道 `bg_599ed2b3`，提醒里却没给。这是提醒归系统最应该修的地方：提醒要带可执行的事实。

#### 缺陷 B：8 次静默 clamp，0 次提醒

`exec_command` 的 `yield_time_ms` 上限是 30,000ms，但模型有 8 个源码调用点写了超过 30 秒：

- 60,000ms × 7
- 120,000ms × 1

`clamp_int` 实际上算出了 requested -> applied 的 note，但 `exec_command` 把它丢掉了，`ExecCommandData` 里也没有 `clamped` 字段。

结果是：模型以为等了 60 秒，实际 30 秒就返回了 live session，没有任何系统提醒告诉它被截短了。

### 3.8 run_code 管线的 `return` / `console` 不一致

43 次代码里：

- `return` 语句：0 次
- `console.log`：63 次
- 42 次输出末尾是 `null`

prelude 说 returned value is the result，但模型 100% 用 `console.log`，从不 `return`。renderer 仍然把 `null` 作为最后一个 entry 打出来。

`null` 本身只有 42 tokens，不贵。不需要额外强调 `return` 语义，只修渲染：没有返回值时不要输出 `null`。

### 3.9 当前 tracing 状态

- `trajectory.json` 的 `tool_calls` 只有外层 `run_code`。
- `sessions.db` 的 `result_data.by_tool` 有嵌套调用的聚合计数。
- 嵌套调用明细只在 `litecode.log` 的 `tool_audit` 里，`call_id` 形如 `parent:code:n`。
- 输入字段 / 输出字段读取 / 提醒分类：需要解析 `run_code` 源码 AST 和 `function_call_output.output`，ATIF 里没有。
- `tool-audit.jsonl` 已经自然写到 `.litecode/diagnostics/<session>/`，但 `run_turn.sh` 没有把它复制到 `/logs/agent/litecode/`，所以试验 artifact 里看不到。

这意味着：当前评测轨迹只能看到外层 `run_code`，看不到 agent 真正调用了哪些嵌套工具、用了哪些参数、读了哪些结果字段。这正是本轮基础设施改造要补的部分。

## 4. 问题

### P1：绑定面太厚

本次 13 个 code tools 只用了 5 个，8 个 0 调用。`todo` 也是 0 调用。

这不是说这些工具永远没用；4 个任务不够，也不能据此从 eval 删工具。它只能说明：产品运行时可以考虑按需绑定；评测必须保留 grep/glob/web 来覆盖工具面。

### P2：结果面把 agent 结果字段和提醒数据 混在一起

`ToolCallResult.data` 同时承担：

1. agent 在 JS 里读的结果字段；
2. renderer 生成提醒用的数据；
3. 前端 `result_data` 的聚合。

实际数据里：

- `path_risk` 产生 5 次，agent JS 读取 0 次，全部由 renderer 消费；
- `diagnostics` 产生 0 次；
- `clamped` 产生 0 次；
- `read.has_more` / `total_lines` 在 4 次 read 里 agent 直接读取 0 次，其中 2 次由 renderer 生成分页提醒。

同一事实表示两遍：公开结果字段 + renderer 提醒。

### P3：提醒缺少可执行事实

- live session 提醒不带 `session_id`，导致后续 `write_stdin` 用错 id。
- clamp 没有提醒，模型不知道 requested/applied 不一致。
- `stderr` 警告重复 3 次；`/tmp/t_edge.py` path risk 重复 2 次。

### P4：固定面重复

- `libs` 598 tokens，0 使用；prose 又把方法列表枚举一遍。
- `edit` / `apply_edits` 两个入口，本次 edit 0 调用，apply_edits 1 调用，签名合计约 336 tokens。
- `run_code.timeout_ms` 0/43 使用。
- `todo` 245 tokens，0 调用。
- `PathRisk` / `ToolDiagnostic` / `ClampedParam` 及可选字段约 425 tokens/请求，提醒文本总量却只有 349 tokens（整场 4 任务）。

### P5：tracing 不是 first-class

当前要回答高频接口、高频输入、agent 读哪些输出、提醒属于谁，必须人工解析：

- `sessions.db` 的 `run_code` 源码；
- `litecode.log` 的 `tool_audit`；
- `function_call_output.output` 的文本；
- AST。

这套分析不能每次评测后手工重做，必须变成评测基础设施的一部分。

### P6：`return` / `console` 行为不一致

模型 0 次 `return`，42 次输出尾行 `null`。`RUN_CODE_VIEW_TOKENS = 10,000` 一次都没触发；当前不是渲染预算问题，而是语义和用法不一致。

## 5. 解决方案

### 5.1 评测基础设施：tool 层结构化审计 + 自动落盘

**不动 `run_code` 实现**，在通用 tool 执行层增加结构化审计字段：

- `src/runtime/diagnostics.rs`：扩展 `AuditEvent`，增加可选字段：
  - `caller`: `direct` / `code`
  - `parent_call_id`
  - `tool_path`（内置工具映射到 SDK path，如 `exec_command` -> `process.exec_command`）
  - `input_keys`: 一级输入字段名（不记录值）
  - `input_bytes`
  - `output_fields`: `result.data` 的一级字段名
  - `content_bytes` / `data_bytes`
  - `duration_ms`
  - `error_code`
- `src/tool/executor.rs`：在已有的 `start` / `exec_done` 审计事件上填充这些字段。所有 direct 和 nested 调用都走同一入口，因此不需要改 `run_code`。
- `eval/harbor/run_turn.sh`：把 `.litecode/diagnostics` 复制到 `/logs/agent/litecode/`，让 `tool-audit.jsonl` 自然进入 trial artifact。
- `eval/harbor/bench5/collect.py`：读取 `agent/litecode/diagnostics/*/tool-audit.jsonl`，按 `call_id` 合并 start/exec_done，输出：
  - `tool_usage.present`
  - `tool_usage.calls_by_tool`
  - `tool_usage.by_caller`
  - `tool_usage.input_keys`
  - `tool_usage.output_fields`
  - `tool_usage.errors`
  - `tool_usage.duration_ms`
  - `tool_usage.content_bytes` / `data_bytes`
- 同时从 `trajectory.json` 的 observation 文本解析 `↳` 行，输出：
  - `tool_usage.reminders.lines`
  - `tool_usage.reminders.outputs_with_reminders`
  - `tool_usage.reminders.by_category`
  - `tool_usage.reminders.by_tool`
  - `tool_usage.reminders.unique_texts`

这样下一次评测结束，`bench5-report.json` 每个 cell 自带 `tool_usage`，`bench5-report.md` 自动带一节工具接口观测；不需要再手工查日志。

### 5.2 工具接口减薄

按优先级：

1. **评测不按 0 调用减工具**：`grep` / `glob` / web 全部保留，评测目标就是覆盖工具面。绑定策略只用于产品运行时或非评测场景，不能拿 eval profile 做动态删减。
2. **结果字段 / 提醒数据分离**：
   - 结果字段只保留 agent 在 JS 里真正读的内容；
   - 提醒数据不进入 JS 结果和 SDK 类型，只给 renderer / telemetry；
   - 先从 `PathRisk`、`ToolDiagnostic`、`ClampedParam`、`has_more` / `total_lines` / `truncated` / `wall_time_seconds` / `original_token_count` 开始。
3. **合并重复入口**：
   - `edit` / `apply_edits` 只保留一个单条/多条都支持的编辑工具；
   - `run` / `wait` / `kill` 已经 cutover 删除，这次 0 调用验证了方向。
4. **清理固定重复**：
   - 清理只在产品运行时做，eval profile 不因此减工具。

### 5.3 提醒修正

- live session 提醒带真实 `session_id`；
- clamp 提醒带 `requested` / `applied`；
- 截断提醒带 `truncated` 或续读方式，不保留 `original_token_count`；
- 按 `(工具, 事实, 目标)` 去重；
- `/tmp` 这类明确允许的路径，path risk 按策略降级或聚合；
- `stderr` 警告按 `(tool, field)` 每次 turn 只报一次。

### 5.4 run_code 语义

- 不额外强调 `return`；
- 只修渲染：没有返回值时不再输出 `null`。

## 6. 理由

### 6.1 为什么从 tool 层做 tracing，而不是改 run_code

- `run_code` 是编排层；它不该为评测统计承担职责。
- 所有 nested 调用都经过 `execute_tool_call`，在 tool 层加字段一次覆盖 direct 和 code 两条路径。
- `tool-audit.jsonl` 已经存在，只是没有带结构化字段、没有被复制进 artifact；改造成本低。
- 不增加模型可见 token，不影响 agent 行为。

### 6.2 为什么不是先删类型

- 本次 4 个任务只能证明这些任务没用，不能证明所有任务没用。
- 类型的最大问题不是绝对 token 数，而是 agent 不读、宿主还要用的层次错位；先分结果字段和提醒数据，再决定哪些类型不再公开。
- 绑定面只在非评测运行时才是杠杆：1,128 tokens/请求（当前 10 工具面）vs 425 tokens/请求（可选 metadata 面）；eval profile 不做这种删减。

### 6.3 为什么提醒要带事实

- 提醒文本只占输出 1.6%，不是成本问题；它的价值在减少往返。
- live session 用错 `session_id` 的例子说明：提醒不带事实，模型仍然要猜，提醒就没有完成它的职责。
- clamp 被静默丢弃的例子说明：宿主已有 requested/applied，却让模型在错误假设下继续运行。

## 7. 落地阶段

### Phase 0：报告与口径（本文件）

- 固化本次复盘数据；
- 明确 pre-cutover / post-cutover 的边界；
- 记录 report.json 与 selected.json 的不一致。

### Phase 1：tool 审计字段 + artifact 落盘（已实现）

- 扩展 `AuditEvent`；
- 在 `execute_tool_call` 填充 caller / parent / input_keys / output_fields / bytes / duration / error_code；
- `run_turn.sh` 复制 diagnostics；
- 不修改 `run_code` / renderer。

验收：

- 新评测 trial artifact 下存在 `agent/litecode/diagnostics/<session>/tool-audit.jsonl`；
- 每个 nested 调用都有 `start` 和 `exec_done`，可由 `call_id` 拼成一条记录；
- direct 调用也能看到 `caller=direct`。

### Phase 2：collector 自动汇总（已实现）

- `collect.py` 解析 `tool-audit.jsonl`；
- `collect.py` 从 trajectory observation 解析 `↳` 提醒；
- 每个 cell 的 JSON 增加 `tool_usage`；
- Markdown 报告增加工具接口观测小节。
验收：

- `bench5-report.json` 的 LiteCode cell 含 `tool_usage.calls_by_tool`、`input_keys`、`output_fields`、`reminders`；
- 缺 diagnostics 的旧 trial 显示 `present: false`，不让报告失败；
- 解析坏行不崩溃，只计数/警告。

### Phase 3：下一次评测验证

- 不重跑当前 4 题，等下一次正式评测；
- 检查 `tool_usage` 与 `result_data.by_tool` 是否一致；
- 检查 ATIF 外层工具数 + 嵌套工具数是否能对齐；
- 检查提醒分类是否覆盖所有 `↳` 行。

### Phase 4：工具面收敛

- 评测 profile 不减工具；grep/glob/web 全部保留；
- 只做接口合并、死字段删除、data/metadata 分流；
- `edit` 合并进 `apply_edits`；
- 不用 0 调用数据从 eval 删工具；该数据只用于判断接口是否该合并或改薄。

### Phase 5：提醒修正

- live session id；
- clamp requested/applied；
- 去重；
- path risk 策略化。

## 8. 本轮范围

- 运行时与评测使用同一套工具实现；eval profile 只作为变量控制面。
- 评测 profile 不减工具；grep、glob、webfetch、websearch 全部保留。
- `libs` 不动。
- 删除 `edit`，编辑统一走 `apply_edits`。
- 结果字段只保留 agent 要用的；提醒数据只放 metadata；renderer 不读 metadata 以外的结果字段，只有 `session_id` 和失败信封这类执行标识例外。
- 提醒由 renderer 生成，工具不写提醒文案。
- 没有返回值时不再输出 `null`；不额外强调 `return` 语义。

## 9. 验收清单

- [x] `AuditEvent` 新字段可选，旧事件仍可反序列化；
- [x] `tool-audit.jsonl` 中 nested 调用有 `caller=code`、`parent_call_id`、`input_keys`、`output_fields`；
- [x] direct 调用有 `caller=direct`；
- [x] `run_turn.sh` 把 diagnostics 复制到 trial artifact；
- [x] collector 对旧 trial 缺 diagnostics 不报错；
- [x] collector 汇总 calls / inputs / outputs / reminders；
- [x] 报告 MD 自动出现工具接口观测；
- [x] Python collector 测试通过；
- [x] Rust focused tests / build 通过；
- [x] tracing 基础设施没有新增 model-visible 输出。

## 10. 附录：本次关键数字速查

- 4 tasks / 1 trial each
- 47 LLM requests
- 43 outer `run_code`
- 76 nested calls
- 61 static nested call sites
- exec_command 57 (75.0%)
- file.write 11 (14.5%)
- file.read 4 (5.3%)
- write_stdin 3 (3.9%)
- apply_edits 1 (1.3%)
- 0 calls: edit / glob / grep / web.fetch / web.search / run / wait / kill / todo
- 14 reminders / 349 tokens / 11 of 43 outputs
- path_risk 5 / unknown_field 3 / live_session 3 / read_page 2 / swallowed_failure 1
- SDK surface: ~3,973 current (10 tools), ~4,590 pre-cutover (13 tools)
- used-only surface: ~2,845 current / ~2,866 pre-cutover
- tools_tokens 5,472 per request x 47 = 257,184 prompt tokens (28.4% of 905,261)
- `libs` 598 tokens / 0 uses（观察项，本轮不动）
- `todo` ~245 tokens / 0 calls
- `run_code.timeout_ms` 0/43
- 0 `return` / 63 `console.log` / 42 trailing `null`
## 11. 目标整改内容（已定）

### 11.1 工具集合

- 删除 `edit`，只保留 `apply_edits`。
- 单条编辑写成 `edits: [ ... ]`，数组长度为 1。
- `apply_edits` 始终返回 `results[]`，单条也是数组，不做单条/多条两种返回形状。
- `write` 保留，继续负责整文件替换。
- 整改后 code tools 从 10 个变为 9 个：
  `apply_edits`、`exec_command`、`glob`、`grep`、`read`、`webfetch`、`websearch`、`write`、`write_stdin`。
  这是删除 `edit` 后的接口数，不是 eval 绑定缩减；评测仍然绑定 grep/glob/webfetch/websearch。

### 11.2 删除字段

以下字段既没有 agent 读取，也没有系统提醒消费，直接删除：

| 工具 | 删除字段 |
|---|---|
| `exec_command` / `write_stdin` | `wall_time_seconds`、`original_token_count` |
| `write` | `created`、`file.bytes`、`file.abs`、`file.hash` |
| `write` 输入 | `create_only`、`expected_hash` |
| `apply_edits` | `nearest_line` |
| `read` | `path` |
| `websearch` | `query` |
| `browser_read` | `message` |

`write` 的 `file` 只剩 `file.path`，整改时拍平成 `path`，不再保留单字段的 `file` 对象。

### 11.3 data / metadata 投影

工具仍然只产出一份事实；投影时分流：

- `data`：agent 结果字段，进入 JS / SDK；
- `metadata`：提醒数据，只给 renderer，不进入 JS / SDK。

| 工具 | `data` 保留 | `metadata` 移入 |
|---|---|---|
| `exec_command` / `write_stdin` | `output`、`exit_code?`、`session_id?` | `truncated`、`clamped` |
| `write` | `path` | `path_risk`、`diagnostics` |
| `apply_edits` | `results[]: {ok, path, reason, detail}` | 每项的 `path_risk`、`diagnostics` |
| `read` | `lines` | `start_line`、`total_lines`、`has_more`、`skipped_in_context`、`lines_capped`、`clamped` |
| `glob` | `paths` | `total`、`truncated`、`outside_workspace`、`empty_reason`、`clamped` |
| `grep` | `hits` | `truncated`、`empty_reason`、`total_hits`、`at_limit_cap`、`lines_truncated`、`context_omitted`、`output_capped`、`clamped` |
| LSP 全部 | `items` / `symbols` / `locations` / `markdown` | `state`、`truncated` |
| `browser_read` | `elements`、`page_id`、`text`、`title`、`url`、`truncated` | 暂不搬，等 eval 真正绑定后再按同一规则处理 |
| `websearch` | `results` | `clamped` |
| `code_search` / `session_search` | 原有结果字段 | `clamped` |

失败分支的 `ok: false`、`reason`、`message` 仍属于 `data`，agent 需要它分支。
提醒不能复述 `reason`，只在程序吞掉 failure 继续跑时补充程序继续了这个后果。

### 11.4 提醒规则

- 每条提醒必须自带执行所需信息：`session_id`、下一个 `start_line`、`requested/applied`、路径等。
- 一条提醒只说一件事。
- 同一个 `(工具, 提醒类型, 目标)` 在一次 `run_code` 内只出现一次。
- 提醒被截断时，先保标识和数值，不保说明文字。
- 值已经出现在程序输出里时，提醒只给动作或后果；值没出现时才带值兜底。
- `live_session` 必须带真实 `session_id`。
- `clamp` 必须带参数名、`requested`、`applied`。
- `stderr` 这类字段只报一次，不按调用重复。

### 11.5 输入面

- 删除 `write.create_only`。
- 删除 `write.expected_hash`。
- 保留 `run_code.timeout_ms`；有不够用的情况。
- `exec_command.workdir`、`max_output_tokens`、`tty`、`yield_time_ms` 保留。
- `read.start_line` / `limit`、`write_stdin.max_output_tokens` 保留。
- 没有真实调用样本的 glob/grep/web/LSP/browser 输入先不动。

### 11.6 实现边界

- 工具仍然只产出一份事实，投影分 `data` / `metadata`。
- `data` 进入 JS / SDK；`metadata` 只给 renderer，不进入 JS / SDK。
- `program_result` 只读 `data` 和 `error`，不能读 `metadata`。
- SDK 类型生成只从 `data` 来，`metadata` 不生成公开类型。
- renderer 只消费 `metadata`；`session_id` 和失败信封是双用途例外，只读 `data` 的原始值，不重复解释语义。
- 一个字段只能属于 `data` 或 `metadata` 一边。
- 双用途只允许 `data` 存值，提醒只讲动作或后果，不重复解释同一个语义。
- 工具自身只产出结构化值，提醒文案仍由 renderer 生成。

这一版目标整改内容以此为准；输入面、输出面、提醒面三层已经分开定稿。

## 12. 整改后评测

整改后按当前 4 题、LiteCode、每题 1 trial 跑了一次（tag `litecode-remediation-r3`，build `af6358bb348d`，dirty）：

- 通过：4/4，和上一份全量报告一致，没有回退。
- 请求：63 -> 57。
- total token：1,449,443 -> 1,176,822（-18.8%）。
- 成本：$0.050744 -> $0.046332（-8.7%）。
- 嵌套工具：`edit` 为 0；`apply_edits` 3、`exec_command` 42、`write` 8、`read` 8、`write_stdin` 2、`grep` 1。
- audit：4/4 trial 有 `tool-audit.jsonl`，117 个 span，468 条事件，坏行 0。
- run_code 输出末尾 `null`：旧 42/43 -> 新 0/53。

完整数据、限流和局限见：

`eval/harbor/bench5/tool-interface-remediation-eval.md`
