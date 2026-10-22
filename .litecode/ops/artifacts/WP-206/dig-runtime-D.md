# WP-206 D组（PTC #6 #7）

## 实现
- #6 `src/ptc/boundary.rs`：新增 `illegal_at(kind, frames)` → `"{kind} at {path_at}"`；bigint/symbol/function 三处改用（根值 `$`，如 `bigint at $.a.b`、`symbol at $[1]`、`function at $.f`）。
- #7 `boundary.rs`：`UndefinedPolicy { Reject, Strip, StripOrNull }` + `from_js_with(value, policy, &mut DroppedUndefined)`；`from_js` = Reject（throw details 等保持严格）。
  - 遍历改为 `Child { Value, Null, Skip }` + `descend()` 循环消化 undefined（无递归），对象属性 undefined → 跳过，数组元素 → null，根 → null；全部被剥的对象正常闭合为 `{}`。
  - `DroppedUndefined { paths(≤10), total }`，`head_line()` → `dropped undefined at a.b, c[2] +N more`（路径去前缀 `$.`；根仍为 `$`）。
- `src/ptc/engine.rs`：工具入参两处用 Strip（静默）；`accept_value` 用 StripOrNull，结果记入 `OutputLedger`（`src/ptc/output.rs`），落到 `Outcome.dropped_undefined`（`src/ptc/mod.rs`，re-export `DroppedUndefined`/`DROPPED_CAP`）。
- `src/tools/code.rs` → `RunReport.dropped_undefined`（`src/tools/code/render.rs`）在 head 第二行输出。
- 文案同步：PRELUDE（`src/tool/code_manifest.rs:28` + 断言）、run_code schema `code` 描述（code.rs）、`docs/ptc/charter.md` I10、`docs/ptc/run-code-remediation-plan.md` §248/§361。**未刷 run_code_sdk.* fixture**（sdk_snapshot 预期红）。

## 测（cargo-D-tests.txt）
- ptc:: 55 过 / 2 红（stack_limit、stack_overflow_stays_sealed —— 基线既有，见 cargo-A-baseline-noA.txt）。新增：undefined_in_result_is_stripped_or_nulled_and_reported、dropped_undefined_list_is_capped_with_more_count、undefined_in_arguments_is_dropped_silently；illegal_output_values / illegal_arguments_skip_host 改为断言带路径（bigint at $.a.b、symbol at $[1]、function at $.f、bigint at $.n[0]）并移除 undefined 拒绝用例；深嵌套 sibling 用例改为 `1n` → `bigint at $[1]`。
- tools::code 48 ✅（新增 render 测 dropped_undefined_line_sits_in_the_head）；tool::code_manifest 4 ✅；run_lock_gate 7 ✅（A 回归）。
- 既有红（非 D 引入）：phase1_gate 3、contract_matrix 1；sdk_snapshot（fixture 未刷，预期）。phase1_gate `run_code_calls_match…` 由此前 "undefined at $" 变为在后续 `result.data.is_none()` 断言处失败（WP-201 数据契约旧断言）。
