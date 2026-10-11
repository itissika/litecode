# Batch 3 · client-ux（协议投影 / serve）实现对照

负责人：Session（协议侧）。基线：`075308b`。修复前提交：`74f2bb1`（多 session Ask）。**未 commit / 未 push。**

## 1. 节点对照表

| key | 结论 | 证据 |
|---|---|---|
| `connection-ask-registry`（pending） | **一致，无实质漂移** | `PendingAsk` + `apply_permission_grant`（`request_id → reply_tx`）；`run_session_loop` 注释/实现：Ask 期间仍直接处理各 session JsonRpc，**无**嵌套 select 把异会话 RPC 丢进 deferred；测试 `mod ask_loop_tests › fn ask_does_not_block_other_session_rpc`；`reject_all_pending_asks` 断线/取消 Deny；未知 grant → stray。**与 Andy 口径一致：一 session 只管自己的 Ask；跨 session 整条 WS 串行 = 错（已在 74f2bb1 修掉，本批不改 connection.rs）。** |
| `serve-bind-auth`（pending） | **一致，无实质漂移** | `validate_serve_bind`；`auth::middleware` + Bearer-only `/api` + query token `/ws`；`tests/serve_bind_auth.rs` `fn bind_auth_matrix` |
| `client-projection`（enabled） | **一致**（未改主张） | 镜像/双游标/乐观仍立；补 `相关：session-model`；契约展开已挂 ask-registry |
| `ux`（framework） | **一致**（未改主张） | 已挂 `serve-bind-auth` |

无结论 B；**无阻断**。

## 2. 正式 issue

| id | grade/layer | status | 处置 |
|---|---|---|---|
| **ISSUE-019**（= DRAFT-CUX-1） | bug / product | **closed** | 前端 Batch3 已关闭。多 session Ask 串行在 **74f2bb1** 修掉；基线测试 `ask_does_not_block_other_session_rpc`。本域核实：**实现已兑现「一 session 只管自己的 Ask」**，不重开、不重复改 `connection.rs`。 |
| **ISSUE-001** | drift / gate | **closed** | 补齐 `tests/client_protocol_project.rs` / `client_protocol_permission.rs` 对 `PermissionAsk`/`PendingPermission`/`PermissionRequest`/`AskReply` 字段；本工作区已改源码（未提交）。 |
| **ISSUE-008** | drift / gate | open | PermissionModal 样式断言；**归前端**，本切片不改。 |

机读：`.litecode/ops/issues/ISSUE-019.json`、`ISSUE-001.json`。

## 3. 待删 / 待改非知识 md

本协议切片无额外「撞车」md 待删清单。UI/样式测试过期见 ISSUE-008（前端）。

## 4. 互引补全

- `connection-ask-registry` → `ask-surface`、`tool-authorization`（相关）
- `client-projection` → `session-model`（相关）
- `ux` → `serve-bind-auth`（Batch2 已挂）

## 5. 本批代码/文档改动（未提交）

- **未改** `src/client_protocol/connection.rs`（Ask 登记已正确）
- `tests/client_protocol_project.rs`、`tests/client_protocol_permission.rs` — ISSUE-001 字段对齐
- `knowledge/contracts/client/{connection-ask-registry,serve-bind-auth}.md` — §7.2 结论
- `knowledge/contracts/client-projection.md` — 仅补相关
- `.litecode/ops/issues/ISSUE-001.json` — closed

## 6. 阻断声明

**无。** ISSUE-019 已 closed；无 arch；serve 安全门主张与实现一致（不触发 B3）。

## 7. Andy 口径对照（Ask）

| 口径 | 核实 |
|---|---|
| 一 session 只管自己的 Ask | 是：pending 表按 request_id；agent 阻塞在 oneshot；连接循环继续 |
| 跨 session WS 串行卡住别人 = 错 | 已修于 74f2bb1；测试 `ask_does_not_block_other_session_rpc`；ISSUE-019 closed |
| 非 accept = reject | `reject_all_pending_asks` → `AskOutcome::Deny`；`cancel_rejects_pending_ask` |
