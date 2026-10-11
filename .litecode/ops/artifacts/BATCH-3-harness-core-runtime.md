# Batch 3 · harness-core（Runtime 切片）实现对照

- 负责人：Runtime
- 基线 HEAD：`075308b`
- 日期：2026-10-12（HKT）
- **不 commit / 不 push**

## 1. 节点对照表

| key | 结论 | 证据（文件+符号+测试/行为） |
|---|---|---|
| item-authority | **一致** | `fn user_message`；`fn death_list_dialect_tokens_absent_from_src` |
| agent-loop | **一致** | `fn run`；`enum TurnOutcome`；`trait AgentDeps`；auto compact `Err`→`TurnOutcome::Error` 终止 turn |
| stream-projection | **一致** | `struct StreamProjection`；`fn settle`；`fn stream_projection_uses_only_the_lifecycle_writes` |
| turn-llm-binding | **一致** | `struct TurnLlmBinding`；`fn resolve_session_llm`；`fn validate_llm_input_capabilities`；压缩 `fn binding_for_agent` |
| platform-knobs | **一致** | `enum ThinkingSpec`/`ContextMode`；`ModelRequest::compact`→`ThinkingSpec::Off`；`fn compact_llm_path_does_not_bypass_unified_provider` |
| turn-lifecycle（父，enabled） | **一致**（不改主张） | 契约展开已挂本切片子节点；L0/seam/取消收尾与 `fn run` 一致 |

无结论 B。无 core 无法拓展 → 不触 B2。

## 2. 正式 issue

| id | 来源 | grade | layer | core | status | blocks | 处置 |
|---|---|---|---|---|---|---|---|
| ISSUE-003 | 种子 | drift | product | 是（session-model） | open | 否 | Session 主责；Runtime 确认 turn/observer 用 `SessionEvent` 词汇 |
| ISSUE-006 | 种子 | drift | gate | 否 | open | 否 | 非本切片；知晓 |
| ISSUE-018 | Batch3 新 | drift | product | 否 | open | 否 | `CompactionFailed` 文案仍写「3 attempts」，空 summary 实为单次；`CompactionCircuitOpen` 似未发射 |
| DRAFT-HC-R1 | Batch2 | — | — | — | **关闭** | — | 产品路径无绕过 catalog 的厂商 thinking 字符串；平台 low/medium/high 属 ThinkingSpec |

机读：`.litecode/ops/issues/ISSUE-018.json`（017 属 model-config 半写入）。

## 3. 待删/待改非知识 md（本域撞车）

| 路径 | 建议 | 理由 |
|---|---|---|
| （无强撞车待删） | — | Runtime 源码树无平行设计 md；ADR `docs/adr/0002-config-gate.md` 与 config-commit 一致，保留 |
| `LitecodeError::CompactionFailed` Display / `ErrorCode::CompactionCircuitOpen` | 待改（走 ISSUE-018） | 文案/死枚举与实现不符；非知识 md |

## 4. 互引补全

- `agent-loop` → 相关 `context-compact`（auto 失败终止与 compact 契约交叉）
- `stream-projection` ↔ `llm-stream-contract`（一流投影 ↔ 流闸）
- `turn-llm-binding` 已有 相关 provider-catalog / config-commit

## 5. 阻断声明

**无。** 无 arch；无结论 B；不触 B1–B5。

## 6. Andy 口径核实：auto/manual compact（砸体验路径）

### auto compact 失败 → turn 必须终止

| 链路 | 证据 |
|---|---|
| L0 | `src/agent/core.rs` `fn run`：`deps.compact_if_needed` 的 `Err(e)` → `return TurnOutcome::Error(e)`，其后不 `call_model` |
| Runtime | `src/runtime/exec.rs` `impl AgentDeps for AgentRuntime › fn compact_if_needed` → `context_pipeline.compact_step` → `CompactPolicy::compact_if_needed`（`CompactionTrigger::Auto`） |
| 收尾 | `src/runtime/mod.rs` `fn finalize_agent_outcome`：`TurnOutcome::Error` → `TurnEndReason::Error` |
| 传输 | `src/llm/reconnect.rs` `fn scope_compaction`：compact 内 `Failed` 外抛为 `Cleared`，避免 `agent/retry` 误醒（单测同文件） |

### manual compact 失败 → 可重试

| 链路 | 证据 |
|---|---|
| 入口 | `src/client_protocol/controller/session_lifecycle.rs` `fn start_manual_compact`：`try_begin_operation(Compact)` 取租约；spawn `CompactPolicy::compact_now` |
| 失败 | `compact_transcript`/`compact_now` `Err` → `emit_compact_failed` / `CompactionStage::Failed`；spawn 结束 `drop(lease)` |
| 可再调 | 租约释放后可再次 `start_manual_compact`；`StructuredError.retryable=false` 仅表示不可 `agent/retry`（协议注释），不是禁止用户再点压缩 |
| 投影 | `controller/mod.rs` Manual+Failed → `push_operation_error(CompactSession, …)` |

### subagent / 不可重试优先挖

- 子 agent turn 走同一 `AgentRuntime`/`fn run`：auto compact 失败同样 `TurnOutcome::Error` 终止该 turn。
- `compaction` 为 Hidden agent（`spawn_contract` 测试夹具），不是可 spawn 的 subagent 目标。
- 空 summary → `LitecodeError::CompactionFailed`（文案过时 → ISSUE-018）；仍走 Failed 生命周期，不继续主模型步。

## 7. kb_check

- 本切片节点：结论已追加；status 仍 pending；无本切片 missing_symbol / tag_unknown。
- 全库：见 model-config artifact；他域仍可能有 error（前端符号），本切片未引入。
