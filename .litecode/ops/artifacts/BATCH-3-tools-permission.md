# Batch 3 · tools-permission 实现对照

负责人：Tools。基线：`075308b`。工作区 `/workspace/litecode`，分支 `ops/team-framework`。**未 commit / 未 push。**

## 1. 节点对照表

| key | 结论 | 证据（文件 + 符号 / 测试） |
|---|---|---|
| `webfetch-ssrf`（pending） | **一致**（修后） | `validate_public_http_url` + `is_non_public_ip`（IPv4-mapped / `is_unicast_link_local` / 198.18/15）；`WebFetchTool::fetch` 每跳校验；`ssrf_rejects_loopback_and_private_ips` 等 4 测绿。ISSUE-012 closed；DNS 重绑定 → ISSUE-021 |
| `websearch`（pending） | **一致** | `WebSearchTool::call_inner` / `search`；无 URL 出站门（刻意） |
| `custom-tool-hub`（pending） | **一致** | `CustomToolHub::{begin_job,push_notice,take_mailbox,restore_mailbox,cancel_job}`；`runtime/exec.rs` drain → `CustomToolSettled`；`custom_auto_turn`。TP-B2-01 关闭 |
| `mcp-connection-pool`（pending） | **一致** | `McpConnectionPool::{start,get_or_create,stop}`；`hub_call` / `call_tool`。配置≠Warm 见 `tool-request-entry`（TP-B2-02 不升 issue） |
| `tool-adapters`（pending） | **一致** | `Tool::execute`；Custom `spawn_blocking`+`result_from_stdout`；MCP `execute` await 池 |
| `tool-authorization`（pending） | **一致** | `authorize`；`check_floor`（write/edit 敏感 + bash 危险）；`evaluate_tool` + `subagent_turns_safe_ask_into_deny` |
| `tool-call-lifecycle`（pending） | **一致** | `function_call_must_not_execute` / `interrupted_outputs`；`run_tool` 取消/超时；`join_tool_handle` |
| `tool-execution-context`（pending） | **一致** | `ToolExecutionContext` + `session_reader` 硬失败；提供≠消费 |
| `tool-identity`（pending） | **一致** | name vs call_id；`McpTool::new` 前缀；`mcp_catalog_id` |
| `tool-input-validation`（pending） | **一致** | `parse_tool_arguments`；未知顶层 Warning / `additionalProperties:false` Error |
| `tool-output-shaping`（pending） | **一致** | `compose_full`；`finalize_tool_call_result` / `materialize_media_parts` / `truncated_tool_result` |
| `tool-request-entry`（pending） | **一致** | `available_tools` / `should_include_in_llm_list` / `build_tool_list`；MCP 配置键≠Warm |
| `tool-result-delivery`（pending） | **一致** | `outputs_from_tool_results`；orphan cleanup / `seal_tail_unanswered_calls` |
| `tool-scheduling`（pending） | **一致** | `partition_tool_calls`；`execute_batch_cancellable`；`cross_session_lock_keys` + `try_acquire` |
| `tool-pipeline`（enabled） | **一致**（未改主张） | 子契约齐；仅补核实段 |
| `tool-call-contract`（enabled） | **一致**（未改主张） | 同上 |
| `tool-control-boundary`（enabled） | **一致**（未改主张） | 理解模型，非完备清单 |
| `tool-implementation-contract`（enabled） | **一致**（未改主张） | 展开 webfetch/websearch/adapters |

**统计：** 结论 A **18** / 结论 B **0**。无 core 节点结论 B。

## 2. 正式 issue

| id | grade/layer | status | 处置 |
|---|---|---|---|
| **ISSUE-011** | drift / gate | **closed** | WHATWG 三斜杠主机前提错误；测试改写为断言 `host_str==Some("no-host")` |
| **ISSUE-012** | bug / product | **closed** | 补 IPv4-mapped / fe80 / 198.18/15；`cargo test --lib -- webfetch::tests::ssrf` 4 passed |
| **ISSUE-021** | suspect / product | **open** | 自 ISSUE-012 拆出：validate DNS 与 reqwest 再解析之间的重绑定窗口 |
| TP-B2-01 | — | **溶解** | custom mailbox 全路径已接线，不另开号 |
| TP-B2-02 | — | **溶解** | 配置≠Warm 为刻意边界，节点已写 |

机读：`.litecode/ops/issues/ISSUE-{011,012,021}.json`。

## 3. 待删 / 待改非知识 md

| 路径 | 建议 | 说明 |
|---|---|---|
| （无本域撞车产品 md 待删） | — | webfetch/websearch/custom/mcp 无第二套并行规范 md |
| `src/tools/descriptions/*`（含 knowledge/custom 等） | **保留** | agent 指引，非知识主节点；与节点不抢「唯一真相」 |

## 4. 互引补全

- `webfetch-ssrf` ↔ `tool-authorization`（相关，已有）
- `websearch` ↔ `webfetch-ssrf` / `tool-adapters`（已有）
- `custom-tool-hub` ↔ `tool-call-lifecycle`（已有）
- `mcp-connection-pool` ↔ `tool-request-entry`（已有）
- 本批未改主张；未新增跨域 `相关：`（无开放缺口）

## 5. 本批代码改动（未提交）

- `src/tools/webfetch.rs` — ISSUE-012 修复 + ISSUE-011 测试改写
- `knowledge/contracts/tools/*.md` + 四个 enabled 父节点 — 追加 §7.2 核实结论
- `.litecode/ops/issues/ISSUE-{011,012,021}.json`

## 6. 阻断声明

**无。** 无 arch；无 core 结论 B；webfetch SSRF 为工具局部防护（不触发 B3）；未触达 B1–B5。

## 7. kb_check

全库：`node .litecode/ops/tools/kb_check.mjs` → `{"ok":true,"errors":0,"warnings":76,"nodes":78}`（2026-10-12 HKT）。
本域无新增 error。`webfetch-ssrf` 对未提交的 `is_non_public_ip`/`validate_public_http_url` 有 `symbol_drift` 警告（改码相对 HEAD，预期；提交后消除）。
