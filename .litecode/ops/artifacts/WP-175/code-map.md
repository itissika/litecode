# WP-175 / ISSUE-064 · code-map（后端中层 plan fill 输入）

- 日期：2026-10-15 HKT
- 分支 tip 当时：`ops/team-framework`（以箱上为准；未 commit / 未 push）
- 范围：只读 `src/` + catalog + 既有 fixtures；本文件仅产物

## 0. 一句话

`commandcode/gpt-6-luna`（及 opencode / opencode-go 同名模型）走 **Responses SSE**（不是 provider 默认的 Chat Completions）。流经 `ResponsesCodec` → `StreamContractGate` / accumulator → `StreamProjection` 写 session 行 → `agent::run` 工具环。最可能「带崩 loop」的是 **流式事件反序列化失败 / 契约终端失败** 变成 `TurnOutcome::Error`（多数不是 Rust panic）；前端「整页刷新」需与 **ErrorBoundary / WS 重挂 / ask 被 Deny** 对齐，未必是 `location.reload`。

## 1. Catalog 接线（Luna）

| 引用 | endpoint | 关键字段 |
|---|---|---|
| `commandcode` provider | `https://api.commandcode.ai/provider/v1` | 默认 `endpoint_type = "chat_completions"` |
| `commandcode/gpt-6-luna` | **覆盖** `endpoint_type = "responses"` | `usage_patch = "map_max_effort_to_xhigh"`；`reasoning.summary = "auto"`；modalities text+image |
| 同名 | `opencode/gpt-6-luna`、`opencode-go/gpt-6-luna` | 同样 Responses + 同 usage_patch |

锚点：

- `src/provider_catalog/default_catalog.toml` · `[[providers]] id = "commandcode"`（~L93）
- 同文件 · `[[models]] id = "gpt-6-luna" provider_id = "commandcode"`（~L557）
- Codec 选择：`src/llm/codec/mod.rs` · `EndpointKind::Responses => ResponsesCodec`
- Legacy 默认：`src/config/global_db/legacy.rs` 仍把 `"commandcode"` 默认成 ChatCompletions（仅迁移路径；运行时以 catalog 模型覆盖为准）

## 2. 端到端流（provider → runtime → session）

```
ModelRequest
  → ResponsesCodec::complete_with_stream_events   (src/llm/codec/responses.rs)
      SSE 行 → parse_stream_event (harden + serde ResponseStreamEvent)
      → forward_stream_event (stream_contract.rs) → on_event(StreamEvents)
  → AgentRuntime::call_model_complete             (src/runtime/exec.rs)
      on_event: observer.StreamEvent + StreamProjection::observe
      结束后 StreamProjection::settle(returned|partial)
  → agent::run                                    (src/agent/core.rs)
      persist_new → execute_tools → …
  → finalize_agent_outcome → TurnCompleted / Error (src/runtime/mod.rs)
  → SessionManager 消费 TurnCompleted；线程 panic 时无 TurnCompleted 走 finish_turn 兜底
      (src/session/manager.rs ~L3122–3147)
```

Chat Completions 旁路（同 provider 下非 Responses 模型如 `gpt-5.6-sol`）：`src/llm/codec/chat.rs` + `chat_synth.rs`（把 delta/`tool_calls` 合成 Responses 事件）。**Luna 主路径不走这里**，但 Command Code 其它模型若混用仍相关。

## 3. 域所有权排序（嫌疑）

| 排名 | 域 | 理由 |
|---|---|---|
| **1** | **provider / llm codec** | Luna 专用 Responses 方言；未知 SSE type、残缺 usage、畸形 function_call 都会在 `parse_stream_event` / `forward_stream_event` 变 `Err`；已有「unknown type still fail」测例 |
| **2** | **runtime（agent loop + projection）** | `call_model` 失败 → `TurnOutcome::Error`；projection `ensure_open` 索引冲突 / settle 失败会合成错误；loop 本身几乎无 panic，是「优雅失败边界」是否够 |
| **3** | **session** | 流中行 begin/update/seal；失败会 `failed` 后 settle 返回 Err；**很少单独「崩」**，多为连带；回退/半密封口与下一轮 replay 相关 |
| FE（WP-176 协作） | client-ux | AgentErrorBoundary / 未收到干净 `turn_finished` / WS 生命周期；`reject_all_pending_asks` → Deny 可表现为 ask_user「跳过」 |

## 4. Top crash / abort 假说（带锚点）

### H1 — Responses SSE 事件反序列化失败（最高优先）

- **机制**：`ResponseStreamEvent` 来自 `async_openai`（`#[serde(tag = "type")]`，**无 other 变体**）。未知 `type`、或 `response.completed.usage` 缺 **必填** `input_tokens_details` / `output_tokens_details` → `parse_stream_event` → `Err`。
- **已有明确行为**：`keepalive` 被跳过；其它 unknown **故意失败**（`keepalive_is_skipped_and_other_unknown_types_still_fail` @ `responses.rs`）。
- **usage harden 缺口**：`responses_harden::fill_token_details` **只填已存在的 details 对象**，不创建缺失的 details；测例 `fill_token_details_only_touches_existing_objects` 已钉死。Luna 虽挂 `map_max_effort_to_xhigh`，对「有 usage 无 details」仍不够。`ResponseUsage`（async-openai）里 details **非 Option**。
- **流已有 delta 时**：`on_broken_stream(..., produced_items=true)` → **立刻 GiveUp**（不可重放，防重复生成）→ `preserve_partial` → agent 持久化 partial + `TurnOutcome::Error`。
- **锚点**：
  - `ResponsesCodec::parse_stream_event` · `src/llm/codec/responses.rs:165`
  - `complete_with_stream_events` 读循环 · 同文件 `:382+`（parse Err 分支 `:479–491`）
  - `on_broken_stream` · `src/llm/codec/http.rs:212`
  - `harden` / `fill_token_details` · `src/llm/codec/responses_harden.rs:18` / `:141`
  - Authority：`src/authority.rs` re-export `async_openai::types::responses::*`
  - Usage 形状：registry `…/async-openai-0.41.1/src/types/shared/response_usage.rs`

### H2 — 工具调用流契约 / 终端 item id（高）

- **机制**：`function_call_arguments.delta` 可在 gate 合成 `output_item.added`；终端 `response.completed` 上 `assert_unique_item_ids` 若同一 provider item id 出现多次 → `Err(Llm(...))`。Luna 长输出 + 多 tool 时若重复 id / 乱序 index，projection 也会：
  - `ensure_open`：`output index belongs to … not …` / item moved index → `state.failed` → `settle` Err。
- **锚点**：
  - `forward_stream_event` / `assert_unique_item_ids` · `src/llm/codec/stream_contract.rs:634` / `:650`
  - Gate 合成 added · 同文件（文件头契约注释 + `ensure_opened`）
  - `StreamProjection::ensure_open` / `settle` · `src/runtime/stream_projection.rs:223` / `:267+`
  - `call_model_complete` settle 合并错误 · `src/runtime/exec.rs:870–887`

### H3 —「崩 loop」实为 TurnError 边界 + 线程 panic 兜底（中，解释 FE 观感）

- **机制**：`agent::run` 对 LLM/工具/`persist_new` 错误一律 `TurnOutcome::Error`（无 unwrap 产品路径）。`finalize_agent_outcome` 发 `TurnEndReason::Error`。若 **on_event 回调 panic**（例如未来对 usage 字段假设过强）或其它未捕获 panic：turn 线程挂 → channel 无 `TurnCompleted` → `session/manager` 记 `"agent turn thread panicked"` + `finish_turn` 兜底。
- FE：**不是**默认 `location.reload`。Agent 面板是 `AgentErrorBoundary` → PanelCrash（关面板）；Preview 才有 `window.location.reload`。Andy 口中的「整页刷新」更像：WS 重连后面板重挂载 / snapshot 重拉 / 或 ErrorBoundary 清空。
- **锚点**：
  - `agent::run` · `src/agent/core.rs:22`（`LlmStreamInterrupted` 封口 `:63–81`）
  - `finalize_agent_outcome` · `src/runtime/mod.rs:885`
  - Turn 线程 join · `src/session/manager.rs:3129–3147`
  - FE：`web/src/dockview/panels/AgentPanel.tsx` · `AgentErrorBoundary`
  - FE turn 错误文案：`web/src/stores/turnStore.ts` · `turnEndNoticeFrom`（`reason === "error"`）

## 5. Session / transcript 中流风险（次要）

- 投影：**同一 call** 内 item 用 provider `item_id` 映射到 **一个 seq**；delta coalesced（80ms）；seal 用 call 返回副本而非中流快照（`stream_projection.rs` 文件头）。
- 写失败不吞：`observe` 把首错存 `failed`，后续事件忽略；`settle` 返回该错 → call 失败（避免 log 与模型输出永久不一致）。
- Agent：`persist_new` 在步骤边界提交；`LlmStreamInterrupted` 会 pad 未完成 FunctionCall 的 interrupted output，避免下一轮悬空 call（`interrupted_outputs`）。
- 风险：半密封口 + 下一轮 Responses replay（reasoning ciphertext / summary）若 Luna 方言挑剔，可能第二步再炸（属 H1/H2 延伸，不单开 session 根因）。

## 6. 建议调查顺序

**串行（先钉证据，再改）**

1. **取 wire**：实机或 Andy 机开 `LITECODE_LLM_WIRE=<dir>`（`src/llm/codec/wire_dump.rs`；`serve.sh --wire`）跑一轮会 tool/长输出的 Luna turn；看 `*.responses.response.sse.jsonl` 最后失败事件的 `type` / `usage` / function_call 形状。
2. **对照 H1**：用抓到的一行丢进 `parse_stream_event` 单测（或临时 fixture under `tests/fixtures/sse/responses/`）；确认是 unknown type vs usage details vs 其它字段。
3. **对照 H2**：若能 parse 到 completed，检查 output item ids / output_index；必要时对 projection 单测复现 index 冲突。
4. **对照 H3**：日志搜 `agent turn thread panicked` / `deserialize ResponseStreamEvent` / `stream terminal carries item id`；确认 FE 是 ErrorBoundary 还是 WS 重连。
5. **再动产品码**（子包）：优先 codec harden / 未知事件策略；其次 projection 容错；runtime 只保证 Error 路径仍发干净 `turn_finished`。

**可并行（不挡 wire）**

- FE 中层（WP-176）：对齐「loop Error / 无 TurnCompleted / Cancel / disconnect」时 `reject_all_pending_asks` → `AskOutcome::Deny` 是否即「未点就跳过」（`connection.rs:1558–1570`）。
- 列一张 Command Code 其它 Responses 模型（`gpt-6.1-sol` 等）是否同样复现 → 判「Luna 独有」vs「CC Responses 族」。

## 7. 建议子包拆分

| 子包 | owner 建议 | 范围 | 依赖 |
|---|---|---|---|
| **WP-175-a · provider** | provider / runtime 兼 codec | wire 复现；`parse_stream_event` / harden：未知 type 策略（跳过 vs 失败）、usage details 补全、必要时 function_call 字段修补；加 SSE fixture + 单测 | 无（先开） |
| **WP-175-b · runtime** | runtime | 确认 Error/Interrupted 路径必发 `TurnCompleted(Error)`；projection 对 Luna 乱序 index 的失败信息可诊断；**不改协议语义除非 a 要求** | 可与 a 部分并行；合入顺序 a→b |
| **WP-175-c · session**（可选） | session | 仅当证明 seal/replay 在第二步放大失败；death_list / 写路径不动则跳过 | 依赖 a 根因 |
| **FE 联动** | frontend_mid（WP-176） | 根因分类含「随 175 loop 崩/结束连带 Deny」；**先对齐再改** | 与 175-a 证据串行对齐 |

中层 fill 时：本包 owner=backend_mid，approver=gm；子包 approver 仍走中层交叉审。

## 8. 最小复现（尽量无真 Luna key）

1. **脚本化（首选）**：扩展 `tests/fixtures/sse/responses/` + `tests/common/responses_replay.rs` / `tests/agent_e2e_responses.rs`  
   - 案例 A：正常 text/tool 流后插入 `{"type":"response.vendor_foo"}` → 期望 GiveUp + partial 或（若改策略）跳过。  
   - 案例 B：`response.completed` 带 `usage: {input_tokens, output_tokens}` **无** details → 当前应 deserialize fail。  
   - 案例 C：重复 item id 的 completed output → `assert_unique_item_ids`。  
   - 本地 HTTP：`serve_responses_queue` / 现有 `fixture_responses_sse("tool_call")` 模式。
2. **实机**：`commandcode/gpt-6-luna` + `LITECODE_LLM_WIRE`；prompt 逼出 ≥1 tool（如 bash/ask_user）+ 中等长输出；保存 wire 进 `.litecode/ops/artifacts/WP-175/wire-sample/`（脱敏）。
3. **对照**：同 prompt 换 `commandcode` 下纯 Chat 模型（如 catalog 默认 chat 的条目）→ 若只有 Responses Luna 挂，坐实 H1/H2。

## 9. 与 WP-176（ask_user 误跳过）的开放问题

1. Andy 复现 Luna 崩 loop 时，**是否同时**有 ask_user 卡？若有，时间线是「先 skip 再崩」还是「崩/结束瞬间卡消失」？
2. `reject_all_pending_asks` 在 Quit/Cancel/部分 JsonRpc/断连路径发 **Deny**（不是 Aborted）。FE 是否把 Deny 渲染成「跳过」？
3. Turn 线程 panic 无 `TurnCompleted` 时，FE `turnStore` 是否卡在 running，随后重连清订阅导致卡卸挂？
4. 后端 175 若只把「硬失败」改成「优雅 TurnError」，是否仍触发 FE 卸卡？若会，176 需独立降级（错误边界不误 Deny）。
5. `reload_session` 仅见于 `web/src/api/types.ts` 联合类型——谁发出、是否会整页感刷新，需 FE 侧确认（后端 map 未找到发射点）。

## 10. 对 fill 的阻塞 / 非阻塞

| 项 | 状态 |
|---|---|
| 域拆分与调查序 | **可填**：上表足够写 goal/done_when/子树 |
| 实锤根因 | **未阻塞 fill**；阻塞的是 **implement**（要 wire 或 fixture） |
| Andy 实机日志 / wire | 未在箱上；fill 可写「首步取证」 |
| 改 src | 中层不做；子包落地 |

## 11. 关键文件速查

| 区域 | 路径 |
|---|---|
| Catalog | `src/provider_catalog/default_catalog.toml` |
| Responses codec | `src/llm/codec/responses.rs`, `responses_harden.rs`, `sse.rs` |
| 流契约 | `src/llm/codec/stream_contract.rs` |
| Chat（非 Luna 主路径） | `src/llm/codec/chat.rs`, `chat_synth.rs` |
| Agent loop | `src/agent/core.rs` |
| Call + projection | `src/runtime/exec.rs`, `stream_projection.rs`, `mod.rs` |
| Session turn join | `src/session/manager.rs` |
| Ask Deny 连带 | `src/client_protocol/connection.rs` · `reject_all_pending_asks` |
| Wire 诊断 | `src/llm/codec/wire_dump.rs` |
| 既有 SSE fixture | `tests/fixtures/sse/responses/`, `tests/agent_e2e_responses.rs` |
| Issue | `.litecode/ops/issues/ISSUE-064.json` |
