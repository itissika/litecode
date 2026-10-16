# 175-a brief → board child **WP-178** (parent WP-175)

**Alias:** 175-a / WP-175-a  
**Board id:** WP-178  
**Owner:** provider  
**Approver:** backend_mid  
**Domain:** model-config  
**Parent:** WP-175 (in_progress; diagnose-only)  
**Artifacts:** `.litecode/ops/artifacts/WP-175/` and `175-a/`

## Kind
Diagnose only. No harden. No `src/` behavior change. No AskOutcome / 权限桥 / reject_all_pending_asks / Aborted / WP-176.

## Goal
实锤或证伪 **H1**：Responses SSE `usage` details / unknown event `type` → codec 反序列化失败 / GiveUp（Luna / gpt-6-luna 流式崩 loop 假说之一）。

## Scope (from WP-175 plan.md)
1. **Wire 抓取**：在可复现路径上开 `LITECODE_LLM_WIRE`（或等价），保全 SSE 片段；失败帧与前后文一并落证据。
2. **只读 codec / 契约路径调研**：Responses SSE 解析、unknown type、usage details → GiveUp / TurnError 的 `file:line` 地图（对照 `code-map.md`）。
3. **SSE fixture 取证**：用既有或最小只读 fixture/脚本回放可疑帧；可写 evidence 脚本，**不得**改运行时 codec/harden 行为。

## Out of scope
- 175-b（runtime Error 边界）— 等 175-a 证据后再建  
- 任何修复 PR / responses_harden 行为变更  
- 权限桥、ask_user、WP-176/177

## Done when
- [ ] wire 或 SSE fixture 证据片段入 artifacts  
- [ ] H1 实锤或证伪结论写清（含边界）  
- [ ] codec/契约路径只读调研含 file:line  
- [ ] 未改 `src/` 行为；未碰 AskOutcome/权限桥  
- [ ] 重型编译/跑测需向 backend_mid 排槽（`LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1` `CARGO_BUILD_JOBS=1`）

## Suggested fill text for provider
See sibling `175-a/plan-draft.md`. Provider: `wp.mjs fill --id WP-178 --as provider --content-file ...` then submit; backend_mid will approve.

## Board note
Child **was** created on board (`wp.mjs new --parent WP-175` → WP-178). This brief is the human alias map 175-a ↔ WP-178.
