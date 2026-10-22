# WP-212 acceptance

代码 tip：见 deliver 提交。分支 `feat/tool-contract-ptc`，未合 main。

## 全量
`cargo test -j 2 --no-fail-fast`：**855 passed / 0 failed / 0 ignored**（lib 567）。flaky 重跑：无。明细见 `cargo-full.txt`。

## 五刀落地
| 刀 | 结果 | 主要位置 |
|---|---|---|
| ① kill 假成功 | 确认死后才 ok；仍活着 → `ok:false` + `reason:still_alive` + `exit_code:null`；Windows 仍只经 `kill_process_tree`→`taskkill /T /F`；wait 对工具杀/UI 杀给 `killed:true` + `[process was killed]`，自然退出两者皆无；新增 `tool_killed`（不设 `user_killed`） | `kill.rs`, `wait.rs`, `command_job.rs`, `agent_jobs.rs` |
| ② unhandled rejection | `set_host_promise_rejection_tracker` 记账；settle 收 logs 前一条 `[warn] unhandled promise rejection (N): …`；不 abort | `ptc/engine.rs` |
| ③ host-call 语法 | `pending==1` → `1 host call was … has been cancelled`；复数保持 were/have | `ptc/engine.rs`, `ptc/tests.rs` |
| ④ 文档 | PRELUDE：unknown parameter 整调用 TypeError、不静默丢；rejection→warn；无 setTimeout/setInterval；kill.md / wait.md 同步；SDK 刷新 | `code_manifest.rs`, `kill.md`, `wait.md`, `run_code_sdk.*` |
| ⑤ `<eval>` | `short_stack` 按名丢 `at <eval>`（与 line≤1 叠加）；仅模型视图，原始 `failure.stack` 不变 | `tools/code/render.rs` |

## 新增 protected（WP-208/protected.txt）
- `src/tools/command_job.rs::still_alive_soft_fail_presents_ok_false_with_null_exit_code`（① 呈现）
- `src/tools/wait.rs::wait_after_tool_kill_sets_killed_and_note`（①）
- `src/tools/wait.rs::wait_natural_nonzero_exit_has_no_killed_flag`（①）
- `src/ptc/tests.rs::unhandled_promise_rejection_warns_without_failing`（②）
- `src/ptc/tests.rs::handled_promise_rejection_does_not_warn`（②）
- `src/tools/code/render.rs::short_stack_drops_eval_frames_by_name`（⑤）

## 改动的 protected 断言（逐条）
| 测试 | 旧断言 | 新断言 | 原因 |
|---|---|---|---|
| `src/tools/kill.rs::kill_stops_process_and_notes_output` | 仅 note + pid 死 | 另断言 `ok` 不为 false、无 `reason`（确认死后成功） | ① |
| `src/ptc/tests.rs::unawaited_calls_are_waited_and_recorded` | 含 `1 host calls were still running` | 含 `1 host call was still running` | ③ |
| `src/ptc/tests.rs::success_cancels_inflight_calls` | 同上复数错句 | 单数 `1 host call was` | ③ |
| `src/tools/code/render.rs::failed_run_keeps_completed_calls_and_separates_program_from_tool_counts` | stack 有 boom、无 libs.js | 另断言模型视图无 `<eval>` | ⑤ |

## 遗留
1. ① Windows 真机 `taskkill /T` / 难杀进程仍靠 Andy 本机验证；CI 仅 Unix 树杀 + `still_alive` 呈现夹具。
2. ⑤ 原始 `failure.stack` 仍可能含 `<eval>`——仅模型视图 `short_stack` 变干净。
3. Chat 面 `ignored unknown parameter(s)` 本包未动（Q10）。
4. board/log/tool_traces 若含他人未提交混改，收尾 ops 由 GM 提交；本包只交干净 WP-212 artifacts。
