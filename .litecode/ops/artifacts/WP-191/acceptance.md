# WP-191 acceptance（中层收尾：cargo 已跑）

对照 plan §7。分支 `runtime/wp-191-request-health` @ `2090cc0`（未 push）。
cargo 由后端中层于 2026-10-18 00:12 HKT 补跑（独占槽：开跑前 MemAvailable ≈6.4GiB，无其他 cargo/rustc；`LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2` + 后台看门狗 MemAvailable<2GiB 即杀；未触发）。

| # | 口径 | 状态 | 证据 |
|---|------|------|------|
| 1 | 要素清单 + 出口契约（plan §3） | **done** — 扩展 `LlmRequestBuilt` Internal+Wire；字段见 plan §3.1 | `plan.md` §3、`diff-summary.md` |
| 2 | ≥1 单测 `thinking_not_refilled`（或等价） | **pass** — `runtime::exec::request_health_tests::thinking_not_refilled_when_summary_drops_foreign_reasoning` ok；`…::thinking_not_refilled_when_messages_wholly_omits_foreign_signature` ok（2/2） | `cargo-request_health_tests.log`（5 passed; 0 failed） |
| 3 | 整段省略 → toast 级；正常续聊不误报 | **pass** — toast：`…::thinking_not_refilled_when_messages_wholly_omits_foreign_signature`、`…::bind_sticky_mismatch_is_toast`；不误报：`…::same_model_continuation_with_own_signature_is_silent`、`…::foreign_strip_with_text_retained_is_debug_not_toast`（request_health_tests 共 5/5 ok）；runtime 另发 `SnapshotNotice` warn | `cargo-request_health_tests.log` |
| 4 | diff 证明未改 strip/compact 核心行为 | **pass** — `strip_foreign_ciphertext` 仅返回值 `usize`→`StripCiphertextStats`，新增只读计数 `reasoning_wholly_omitted`，清密文条件/动作不变；`strip_rebased_signatures`、`retain_own_reasoning`、compact 未改；replay_compat 11/11 仍绿 | `diff-summary.md`、`cargo-replay_compat.log`（11 passed; 0 failed） |
| 5 | 合入 main + push origin/main | **pending** — 待 GM 验收后由中层执行（合 main → push origin/main → 工作树切回 main） | — |

## 测试命令

- `LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2 cargo test --lib request_health_tests -- --nocapture` → 5 passed / 0 failed / 2094 filtered（crate `litecode`，模块 `src/runtime/exec.rs::request_health_tests`）
- `LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2 cargo test --lib replay_compat` → 11 passed / 0 failed / 2088 filtered
- 注：早先 `cargo-request-health.txt`（Runtime 跑）只含 replay_compat 11 测，未覆盖 request_health_tests；本次补齐。

## 观测缺口（已知，GM 已接受）

- **`projection_rewrote`:** 本包固定 `false`（exec.rs 显式 `let projection_rewrote = false;`）。view 构建处打标成本高且不得改投影语义；**GM 已接受为已知观测缺口**，后续包可补。
- **`set_model_busy`:** 仍经既有 `operation_error` / `AgentAlreadyRunning`；本包未改拒写逻辑（plan §5）。

## Demo（可选）

跨 provider 历史 reasoning 实机切模型，确认 wire `LlmRequestBuilt.severity=toast` + FE `SnapshotNotice` toast（未做，非验收必需）。
