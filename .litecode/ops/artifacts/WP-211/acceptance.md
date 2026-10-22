# WP-211 acceptance

代码 tip：f83fc91。分支 `feat/tool-contract-ptc`，未合 main。

## 全量
`cargo test -j 2 --no-fail-fast`：**849 passed / 0 failed / 0 ignored**（lib 561）。flaky 重跑：无。明细见 `cargo-full.txt`。

## 三刀落地
| 刀 | 结果 | 主要位置 |
|---|---|---|
| ① write conflict/stale → ok:false | `soft_failure_reason` 纳入 `conflict`\|`stale`；`stale_cursor` 仍 reject；write.md / PRELUDE / `ToolFailure.reason` 同步；SDK 刷新 | `src/tool/outcome.rs`, `src/tools/descriptions/write.md`, `src/tool/code_manifest.rs` |
| ② MSYS pathconv | 呈现层：`message=Some(hint)` ⇒ `ok:false` + `reason:invalid_args` + `exit_code:null`，保留 `message`/`shell`/stdout/stderr；禁止数字 `exit_code:0`；Linux CI 测 present 分支 | `src/tools/command_job.rs`, `src/tools/descriptions/run.md` |
| ③ PRELUDE LSP 七名 | 删除硬编码导航句；描述只经传入 tool blocks；`desired=false`/未传入时描述与 whitelist 均无 LSP 绑定/类型；**未**改 `builtin_code_surface_tools` / Ready+bind | `src/tool/code_manifest.rs` |

## 新增 protected（WP-208/protected.txt）
- `src/tools/command_job.rs::msys_pathconv_hint_presents_ok_false_with_null_exit_code`（②）
- `src/tool/code_manifest.rs::unregistered_lsp_tools_do_not_appear_in_the_description`（③）
- `tests/run_code_model_view.rs::write_conflict_resolves_ok_false`（①）

## 改动的 protected 断言（逐条）
| 测试 | 旧断言 | 新断言 | 原因 |
|---|---|---|---|
| `src/tool/outcome.rs::domain_failures_resolve_ok_false_and_system_failures_reject` | `stale` 在系统失败组里 `is_err` | `conflict`/`stale` resolve `{ok:false,reason}`；`stale_cursor` 仍 reject | ① Q5 |
| `tests/run_code_model_view.rs::uncaught_tool_error_names_the_call` | create_only conflict → `uncaught ToolError conflict … write path=taken.txt` | 读 PNG → `uncaught ToolError unsupported … read path=pic.png` | ①：conflict 改软失败后，throw 命名意图改用仍 reject 的 `unsupported` |
| `tests/code_surface.rs::conflicting_spellings_have_one_create_only_winner_and_a_read_waits_for_the_write` | `allSettled` 以 `rejected` 计 loser | 以 `ok===false` 亦计 error，并从 `message` 取「already exists」 | ① soft-fail 后双方均为 fulfilled |

## 遗留
1. ② Windows 真机 pathconv（MINGW + `cmd /c`）仍靠 Andy 本机验证；CI 仅夹具测呈现分支。
2. SDK fixture 全量仍含 LSP 七工具名（builtin 未删）——验收看「未准入列表」回归，勿误判 fixture 有名即失败。
3. board/log 若含他人未提交混改，收尾 ops 由 GM 提交。
