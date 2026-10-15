# WP-175 / ISSUE-064 方案（backend_mid）— 仅锁定，禁止修复
输入：.litecode/ops/artifacts/WP-175/code-map.md
口径（Andy/GM 2026-10-15）：与 WP-176 无关；禁止任何修复代码；不改 AskOutcome / 权限桥 / Aborted；不写 src 行为变更。

## 目标
稳定复现或证伪 Luna（gpt-6-luna / Responses SSE）流式崩溃；产出根因证据与边界说明；锁定结论交 GM。

## 根因假说（只取证，不修）
- H1 provider：Responses SSE codec 对 usage details 等字段缺失 → 反序列化失败 / GiveUp。
- H2 stream_contract：重复 item id / output_index → 契约终端失败。
- H3 runtime：优雅 TurnError vs panic 的边界需日志/栈区分。
- 取证手段：LITECODE_LLM_WIRE 抓 wire，或 SSE fixture / 只读调研脚本；可写 evidence 脚本，不改 src 行为。

## 子包（可选；批准后建；一律诊断/fixture/wire）
1. 175-a provider：wire 抓取 + 只读 codec/契约路径调研 + SSE fixture 取证（不 harden、不改行为）。
2. 175-b runtime：Error 边界 / 投影路径的诊断日志与栈采集（只读/证据，不修投影逻辑）。

不做：权限桥、ask_user、reject_all_pending_asks、AskOutcome、Aborted、WP-176 相关任何项。

## 边界
- 本包不交付修复 PR；不改 `src/` 运行时行为。
- 前端整页 reload：若复现中观察到，只记录现象与是否伴随 WS/ErrorBoundary；不假设与 WP-176 因果。
- 若证据指向某域修复，锁定报告写清建议，由 GM 另立修复包。

## 验收（锁定完成）
- 有稳定复现步骤（或明确证伪路径）；
- 根因证据：日志 / 栈 / 协议片段 / wire 或 fixture；
- 产物说明边界与「不修」结论；
- 重型取证若需编译/跑测：走后端中层排槽（LC_MIN_MEM_GIB=2 LC_KILL_MEM_GIB=1，CARGO_BUILD_JOBS=1）。
