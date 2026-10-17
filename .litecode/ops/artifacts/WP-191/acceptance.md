# WP-191 acceptance (Runtime draft — pending cargo)

对照 plan §7。**cargo 未跑**；测通过后由中层/GM 合入 push。

| # | 口径 | 状态 |
|---|------|------|
| 1 | 要素清单 + 出口契约（plan §3） | **done** — 扩展 `LlmRequestBuilt` Internal+Wire；字段见 plan §3.1 |
| 2 | ≥1 单测 `thinking_not_refilled`（或等价） | **coded** — `thinking_not_refilled_when_summary_drops_foreign_reasoning` + Messages wholly-omit 测；**待 cargo** |
| 3 | 整段省略 → toast 级；正常续聊不误报 | **coded** — toast vs silent/debug 对照测；runtime 另发 `SnapshotNotice` warn |
| 4 | diff 证明未改 strip/compact 核心行为 | **yes** — 仅旁路 `StripCiphertextStats.reasoning_wholly_omitted`；`strip_rebased_signatures` / compact 未动 |
| 5 | 合入 main + push origin/main | **blocked** — feature 分支 only；合入由中层/GM |

## 观测缺口

- **`projection_rewrote`:** 本包固定 `false`。view 构建处打标成本高且不得改投影语义；记为观测缺口，后续包可补。
- **`set_model_busy`:** 仍经既有 `operation_error` / `AgentAlreadyRunning`；本包未改拒写逻辑（plan §5）。

## Demo（槽放行后）

1. `LC_MIN_MEM_GIB=2 cargo test --lib request_health_tests -- --nocapture`
2. 可选：跨 provider 历史 reasoning 实机切模型，确认 wire `LlmRequestBuilt.severity=toast` + FE `SnapshotNotice` toast。
