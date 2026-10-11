# Batch 2 · model-config 目录清单

- 负责人：Runtime（GM 指派兼管）
- 基线 HEAD：`075308b`
- 作用域：`src/config`、`src/llm`、`src/provider_catalog`

## 1. 拟新增节点

| key | 路径 | 上层 | A# | 一句话承诺 |
|---|---|---|---|---|
| provider-catalog | knowledge/framework/provider-catalog.md | config | A3+A7 | 内嵌种子是唯一 provider/model 来源；按 EndpointKind 选 codec；不是可写设置文档 |
| llm-provider-surface | knowledge/contracts/llm-provider-surface.md | provider-catalog | A1+A4 | 唯一产品入口 complete_with_stream_events；方言关在 codec/；无平行非流式入口 |
| llm-stream-contract | knowledge/contracts/llm-stream-contract.md | llm-provider-surface | A1 | delta 前必须有 output_item.added；forward_stream_event 唯一 choke；中断 seal incomplete |

均为 `status: pending`，`domain: model-config`，含符号证据与 `基线：075308b`。

## 2. 拟保留 / 轻改的存量节点

**保留：**
- framework/config（意图明确）
- contracts/config-commit（已覆盖唯一写门、generation、活动 turn 拒写、目录是构建事实）

**本轮轻改：**
- `config`：契约展开 增补 → provider-catalog（已 append_log）

**不新建（避免与 config-commit 争主）：**
- TurnGuard / 活动 turn 拒写 → 已在 config-commit 主张内；证据点 `src/config/turn_guard.rs` 留 Batch 3 补进 config-commit，不另立主节点
- ResolvedConfig 分层 Builtin<Global<Workspace → 已在 framework/config
- settings_writer 实现细节 → 排除项

## 3. 疑似主节点重复

- 本域三新节点之间：kb_check **无** dup_primary_suspect
- 与存量 config / config-commit：无共享符号证据冲突
- 与 harness-core 的 turn-llm-binding / platform-knobs：仅 `相关：` 指向 provider-catalog，不复述主张

## 4. Issue 草稿（本域）

| 草稿 id | grade | layer | core | basis | title | 证据线索 |
|---|---|---|---|---|---|---|
| （沿用）ISSUE-005 | drift | product | 否 | A7 | `repair_agent_models_on_boot` 在 global_db 外持有 rusqlite::Connection | src/config/manager.rs；session_data_sql_death_list |
| DRAFT-MC-1 | suspect | product | 否 | A5 | 跨介质（文件+DB）提交非事务：失败后部分写入可见性是否有用户可感知不一致 | config-commit 已声明「不承诺错误必无变更」；Batch 3 找复现或降为文档承诺确认 |
| DRAFT-MC-2 | suspect | gate | 否 | A1 | llm reconnect Failed→Cleared（compaction）是否有测试锁 | src/llm/reconnect.rs `scope_compaction`；缺则记 gate 缺口 |

无 arch；不触达 B1–B5。ISSUE-005 建议 Batch 3 优先核实（局部 SQL 边界越界）。

## 5. kb_check（本域）

- 退出码：**0**；errors=0（修过一处：trait 方法无 body 时 tree-sitter 不建链，改为 `impl LlmProvider for ResponsesCodec › fn complete_with_stream_events`）
- 本域 3 个 pending 节点：无 missing_symbol / filename_mismatch / domain_unknown
- config → provider-catalog 的 inactive_target 为预期

## 6. 节点计数（本域新增）

model-config 新增 pending：**3**（provider-catalog, llm-provider-surface, llm-stream-contract）
