# Batch 3 · model-config 实现对照

- 负责人：Runtime（兼管）
- 基线 HEAD：`075308b`
- 日期：2026-10-12（HKT）
- **不 commit / 不 push**

## 1. 节点对照表

| key | 结论 | 证据 |
|---|---|---|
| config（enabled） | **一致** | 唯一写门 / 分层 / 投影不回写；ADR 0002；契约展开含 provider-catalog |
| config-commit（enabled） | **一致** | `commit_partial` / `commit_mixed` / `TurnGuard::is_turn_in_progress`；半写入边界与主张一致（见 §6） |
| provider-catalog | **一致** | `fn embedded`；`enum EndpointKind`；`fn death_list_provider_catalog_is_the_only_llm_source` |
| llm-provider-surface | **一致** | `trait LlmProvider`；Responses/Chat `complete_with_stream_events`；`fn provider_from_model` |
| llm-stream-contract | **一致** | `fn forward_stream_event`；`struct StreamItemAccumulator`；`fn mark_items_incomplete`；`fn scope_compaction` |

无结论 B。

## 2. 正式 issue

| id | 来源 | grade | layer | core | status | blocks | 处置 |
|---|---|---|---|---|---|---|---|
| **ISSUE-005** | 种子升级 | drift | product | 否 | open | 否 | `repair_agent_models_on_boot` 在 `manager.rs` 持有 `rusqlite::Connection`，越出 `global_db/` |
| **ISSUE-017** | DRAFT-MC-1 | suspect | product | 是（config-commit） | open | 否 | `commit_mixed` 先写文件后写 DB；建议加固（非日常写崩） |
| DRAFT-MC-2 | Batch2 | — | — | — | **关闭** | — | `scope_compaction` Failed→Cleared 已有单测 |
| DRAFT-HC-R1 | （交叉） | — | — | — | **关闭** | — | 见 harness-core-runtime artifact |

机读：`.litecode/ops/issues/ISSUE-005.json`、`ISSUE-017.json`。

> 编号说明：013–016 已被上下文/前端等域占用；本域半写入用 **ISSUE-017**。若发现 JSON 碰撞，以 domain 字段为准，由 GM 裁定。

## 3. 待删/待改非知识 md

| 路径 | 建议 | 理由 |
|---|---|---|
| `docs/adr/0002-config-gate.md` | **保留** | 与 config-commit 一致，非过时 |
| `src/config/guides/*.md` | 保留（产品内指南） | 非知识主张撞车 |
| `src/config/global_db/prompts/**` | 保留 | 运行时提示词，非设计 md |

## 4. 互引补全

- `llm-provider-surface` → 相关 `turn-llm-binding`
- `llm-stream-contract` → 相关 `stream-projection`
- `config-commit` 依据补 `commit_mixed` / `TurnGuard`（不改主张）
- `config` 补 `domain : model-config` + 基线 + 核实结论

## 5. 阻断声明

**无。**

## 6. Andy 口径核实：config-commit 半写入

**真相（有证据）：**

- `SettingsWriter::commit_mixed`（`src/config/settings_writer.rs`）：持 `commit_lock` → `write()?`（工作区文件）→ mutate settings → `store::replace_all`（DB）→ `revision.fetch_add` → broadcast。
- 若 `write()` 成功而随后 validate/DB 失败：文件已变、DB/generation 未变 → 跨介质半写入可见。
- 调用点：`delete_workspace_custom_tool`、`delete_workspace_mcp_server`（文件删 + 剥 agent 工具绑定）。
- 节点主张已写清：「跨介质写入不是事务，不承诺错误必无变更」。**结论：一致**，不是未记载的日常写崩。

**建议（ISSUE-017，可选加固，不改契约主张）：**

1. DB/`replace_all` 成功后再落盘，或写入 temp + 成功后 rename；或 DB 失败时回滚文件。
2. 或保持现状，仅在节点/运维文档强调该边界（已满足 Andy「节点写清或实现加固均可」的「写清」侧）。

## 7. llm reconnect / compact（model-config 侧）

- 产品面仍唯一 `complete_with_stream_events`；compact 经 `ModelRequest::compact` + 同入口。
- `scope_compaction` 保证 compact 传输耗尽不留下可 `agent/retry` 的 Failed（转为 Cleared）。
- auto/manual 行为细节见 `BATCH-3-harness-core-runtime.md` §6。

## 8. kb_check

- 本域节点：无本域 missing_symbol / tag_unknown（结论区已避开非法关系标签冒号）。
- 全库 errors：他域（前端 ask-surface/layout-ssot/theme-tokens 符号）可能仍 >0；本域未引入。
