# WP-167 本地补验（转 Andy；本机直接用 cargo，不需要 cargo_locked.sh）
状态：`cargo_locked.sh check --bins -j1` 通过（错误 0，Send 无错，峰值 RSS 1.36 GiB，运行期最低 MemAvailable 5.29 GiB）。以下测试**均未跑**（内存不足/转本机）。仓库根目录执行，一次一条。

| # | 命令 | 预期 |
|---|---|---|
| 1 | `cargo test --lib lsp::` | 全过。含新增 sweep_gate 4 条（wait_passes_when_never_armed / wait_blocks_until_sweep_returns / gate_opens_when_sweep_panics / second_start_is_noop_after_open）与 lsp/mod.rs 5 条探测与门闩测试（Unix-only，见下） |
| 2 | `cargo test --lib engines::` | 全过。含 4 条 lsp 探测测试（Unix-only）、path_index 的 `legacy_text_index_removed_by_worker_after_fill` 与改过的 `attach_fills_explorer_corpus_and_second_attach_does_not_restart` |
| 3 | `cargo test --lib optional::` | 全过（Tools 的 tools.diff 已打入）：defer_reconcile_builds_no_client、ensure_warm_builds_once_and_reuses、concurrent_first_calls_build_once、failed_build_is_not_cached_and_retries、stopped_engine_is_not_rebuilt、reenable_after_stop_builds_on_next_call、settings_reconcile_rebuilds_immediately，以及既有 reconcile_starts_warmup_for_network_core |
| 4 | `cargo test --lib tools::` | 全过。含 websearch 的 failed_warmup_returns_the_real_error、call_async_runs_on_the_blocking_pool |
| 5 | `cargo test --test startup_order_death_list` | 8 条。`serve_run_does_not_sweep_inline`、`serve_run_does_not_warm_optional_engines` 已对照 run.rs 现状论证应满足（run.rs 已含 start_background 与 defer_reconcile）；其余 6 条应过 |
| 6 | `cargo test --test optional_lsp` | 回归：沿用旧的 `hub.activate`（已保留的急切探测），应与改前一致；依赖 python3 mock server（tests/fixtures/mock_lsp_server.py） |

## 最大风险：测试依赖 sh，Windows 上会失败
读代码核对：依赖 `sh` 脚本的测试都已标 `#[cfg(unix)]`（Windows 上整体不编译、不运行，不会失败）：
- src/lsp/mod.rs（辅助 `fake_ls`、`calls` 与 5 条）：deferred_activate_runs_no_probe_until_first_need、probe_runs_once_per_activation、probe_failure_deactivates_hub_and_reports_original_error、restart_after_failed_probe_says_hub_not_active、spawn_ls_waits_for_sweep_gate
- src/engines/mod.rs（辅助 `broken_ls`、`eager_error_prefix` 与 4 条）：lsp_first_need_probe_failure_sets_failed_same_error、lsp_probe_is_skipped_unless_warm、lsp_probe_failure_after_stop_does_not_resurrect_state、detail_view_after_deferred_probe_matches_eager
无缺失标注，**未改代码**。其余新增测试（sweep_gate、path_index、startup_order_death_list、optional/tools）不依赖 sh，Windows 可跑。
- 若 Andy 本机是 Windows：上面 9 条在 Windows 上被跳过，**探测延后、失败回写 Failed、先 sweep 后 spawn 这几条核心行为在 Windows 上无自动覆盖**。需要的话另开跨平台版本（用 `cmd /c exit 3` 之类的 .cmd 脚本）；本包未做。
- 另：`optional_lsp` 需要 python3，Windows 上需有 `python3` 命令。
- death-list 用 include_str! 读源码，CRLF 不影响（按行/括号扫描）。

## check --bins 的 3 处警告核对（日志 /tmp/check2.log）
均**非 WP-167 引入**（符号不在 ide.diff / tools.diff 中，git log -S 指向 Initial public release v0.1.3 或更早的 3794d37）：
- src/lsp/server.rs:232 `mark_failed`/`configuration_response`/`send_request_synced` never used：既有，未改。
- src/tools/bash.rs:47 `with_foreground_wait` never used：既有（Tools），未改。
- src/tools/webfetch.rs:133 `validate_public_http_url` never used：既有（Tools），未改。
（另 src/workspace/agent_path.rs 的 `UnixStyleOnWindows` 与 engines/mod.rs:437 `workspace_root` 等也是既有。）本次**没有改任何代码**，无新 diff，无需重编。
