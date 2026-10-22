# WP-208 deletions（按保护集机械删除）

规则：删除范围 = 各域范围内测试 − protected.txt（守护 665 + 回归 183，去重 764）− 范围外测试；删整个测试函数，连带只被这些测试用到的测试 helper / import（仅限 `#[cfg(test)]` 区域与 tests/，`tests/common` 不动），不改产品代码、不改写测试。GM 决议：`src/serve/run.rs` case_a..d 删除。

合计删除 1900 条（源码计）。

## 按域

| 域 | 删除 |
|---|---:|
| Runtime | 144 |
| Session | 458 |
| 上下文 | 473 |
| Provider | 162 |
| IDE | 238 |
| Tools | 425 |

## 按类别（按测试名关键词归类，首个命中为准，为近似统计）

| 类别 | 数量 | 代表例子 |
|---|---:|---|
| 序列化/wire 形状/roundtrip | 130 | `src/ptc/boundary.rs::json_bytes_counts_a_deep_array_on_the_heap`<br>`src/ptc/boundary.rs::json_bytes_matches_serde_for_shallow_values`<br>`src/ptc/json.rs::shallow_clone_eq_and_debug_match_serde`<br>`src/ptc/tests.rs::moderate_nesting_round_trips`<br>`src/ptc/tests.rs::host_safe_integer_round_trips` |
| 文案/描述/提示文本 | 177 | `src/ptc/tests.rs::console_formats_errors`<br>`src/runtime/exec.rs::thinking_not_refilled_when_messages_wholly_omits_foreign_signature`<br>`src/runtime/exec.rs::text_only_step_stops`<br>`src/runtime/exec.rs::foreign_strip_with_text_retained_is_debug_not_toast`<br>`src/runtime/llm_resolve.rs::document_file_is_allowed_under_text` |
| 入参/schema 校验边角 | 225 | `src/runtime/exec.rs::empty_output_stops`<br>`src/runtime/llm_resolve.rs::preview_rejects_a_model_outside_the_catalog`<br>`src/runtime/llm_resolve.rs::video_file_is_rejected_without_the_capability`<br>`src/runtime/llm_resolve.rs::projection_replaces_unsupported_parts_then_validates`<br>`src/runtime/llm_resolve.rs::an_unknown_reference_is_a_missing_model_error` |
| 配置/预设/目录清单 | 150 | `src/runtime/exec.rs::same_model_continuation_with_own_signature_is_silent`<br>`src/runtime/llm_resolve.rs::compaction_binding_uses_its_own_provider_and_credentials`<br>`src/runtime/llm_resolve.rs::a_switch_to_a_tool_less_model_is_blocked_before_persisting`<br>`src/runtime/llm_resolve.rs::a_session_is_left_alone_when_no_model_is_selectable`<br>`src/runtime/llm_resolve.rs::a_provider_without_a_key_is_not_ready` |
| 死名单/架构源码扫描 | 25 | `src/runtime/stream_projection.rs::an_item_the_stream_never_announced_opens_on_its_first_delta`<br>`src/config/global_db/legacy.rs::an_ambiguous_endpoint_match_is_never_guessed`<br>`src/config/global_db/migrate.rs::v6_upgrade_is_additive_and_never_drops_legacy_rows`<br>`src/session/manager.rs::channel_observer_never_tags_parent_session_id`<br>`src/session/surface.rs::control_plane_kinds_are_absent_from_both_views` |
| 索引/检索/排序细节 | 166 | `src/terminal/shell.rs::shell_command_embeds_payload`<br>`tests/run_code_model_view.rs::cjk_result_does_not_panic`<br>`src/config/global_db/mod.rs::leftover_websearch_endpoint_meta_is_ignored_and_kept`<br>`src/serve/run.rs::case_d_require_auth_without_token_fails`<br>`src/serve/run.rs::case_b_non_loopback_require_auth_with_token_ok` |
| 并发/取消/超时的重复变体 | 49 | `src/ptc/tests.rs::concurrent_host_calls`<br>`src/runtime/phase.rs::t5_cancelling_wrapper_aborted_leave_cancelling`<br>`src/runtime/phase.rs::t4_aborted_leave_cancelling`<br>`tests/agent_loop.rs::cancelled_breaks_loop`<br>`tests/subagent_tool_pipeline.rs::launch_declares_pipeline_parallel_and_cancellable` |
| 其他功能细节/happy path | 978 | `src/agent/core.rs::stop_check_breaks_without_pending`<br>`src/ptc/budget.rs::paused_wait_is_not_charged`<br>`src/ptc/tests.rs::microtasks_do_not_drop_host_calls`<br>`src/ptc/tests.rs::type_error_from_host_is_catchable`<br>`src/ptc/tests.rs::in_flight_limit` |

逐条清单见各域 commit 的 diff（d35f5f8、2d43fd5、b00bde0、af0fa51、333a9d6、55dafa9、fa4c3e4）。

## 按文件删除数（3acfe31 → 收尾提交，按 `#[test]`/`#[tokio::test]` 属性计数）

| 文件 | 前 | 后 | 删除 |
|---|---|---|---|
| `src/session/data/sqlite/session.rs` | 74 | 10 | 64 |
| `src/engines/session_search/semantic_index.rs` | 46 | 3 | 43 |
| `tests/settings_api.rs` | 46 | 6 | 40 |
| `src/engines/mod.rs` | 34 | 0 | 34 |
| `src/provider_catalog/tests.rs` | 35 | 6 | 29 |
| `src/engines/session_search/mod.rs` | 34 | 7 | 27 |
| `src/llm/codec/responses.rs` | 37 | 10 | 27 |
| `src/llm/codec/messages.rs` | 33 | 8 | 25 |
| `src/session/manager.rs` | 45 | 20 | 25 |
| `tests/client_protocol_project.rs` | 26 | 2 | 24 |
| `src/engines/session_search/sparse.rs` | 23 | 0 | 23 |
| `src/tools/grep.rs` | 30 | 7 | 23 |
| `src/workspace/watcher.rs` | 25 | 4 | 21 |
| `src/config/settings_writer.rs` | 27 | 7 | 20 |
| `src/engines/code_search/enclosing.rs` | 20 | 0 | 20 |
| `src/ptc/tests.rs` | 49 | 30 | 19 |
| `src/tools/code_search.rs` | 24 | 5 | 19 |
| `src/engines/code_search/lexical.rs` | 18 | 0 | 18 |
| `src/session/snapshot.rs` | 25 | 7 | 18 |
| `src/tools/read.rs` | 26 | 8 | 18 |
| `src/config/workspace.rs` | 19 | 2 | 17 |
| `src/lsp/install/github_tests.rs` | 18 | 1 | 17 |
| `src/tool/schema_validate.rs` | 20 | 3 | 17 |
| `src/engines/session_search/query_plan.rs` | 16 | 0 | 16 |
| `src/llm/codec/chat.rs` | 22 | 6 | 16 |
| `src/llm/port.rs` | 16 | 0 | 16 |
| `src/lsp/mod.rs` | 27 | 11 | 16 |
| `src/session/transcript_file.rs` | 15 | 0 | 15 |
| `src/tool/executor.rs` | 21 | 6 | 15 |
| `src/tools/custom.rs` | 18 | 3 | 15 |
| `src/tools/glob.rs` | 19 | 5 | 14 |
| `src/tools/litecode_workspace.rs` | 14 | 0 | 14 |
| `src/tools/webfetch.rs` | 22 | 8 | 14 |
| `src/workspace/filter/workspace_excludes.rs` | 14 | 0 | 14 |
| `tests/code_surface.rs` | 34 | 20 | 14 |
| `src/client_protocol/controller/mod.rs` | 15 | 2 | 13 |
| `src/runtime/llm_resolve.rs` | 13 | 0 | 13 |
| `tests/stage_a_ctx_consistency.rs` | 25 | 12 | 13 |
| `src/lsp/deps.rs` | 14 | 2 | 12 |
| `src/tool/availability.rs` | 12 | 0 | 12 |
| `src/workspace/tree.rs` | 12 | 0 | 12 |
| `src/engines/code_search/mod.rs` | 13 | 2 | 11 |
| `src/engines/code_search/reconcile.rs` | 12 | 1 | 11 |
| `src/tools/plan.rs` | 12 | 1 | 11 |
| `src/tools/todo.rs` | 11 | 0 | 11 |
| `src/tools/write.rs` | 16 | 5 | 11 |
| `src/types/death_list_gate.rs` | 13 | 2 | 11 |
| `src/workspace/text_codec.rs` | 11 | 0 | 11 |
| `src/config/manager.rs` | 11 | 1 | 10 |
| `src/context_pipeline/keep_recent.rs` | 15 | 5 | 10 |
| `src/engines/code_search/index_status.rs` | 13 | 3 | 10 |
| `src/engines/session_search/ranking.rs` | 10 | 0 | 10 |
| `src/session/surface.rs` | 10 | 0 | 10 |
| `src/tool/registry.rs` | 17 | 7 | 10 |
| `src/tools/knowledge.rs` | 10 | 0 | 10 |
| `src/tools/session_search.rs` | 14 | 4 | 10 |
| `tests/optional_lsp.rs` | 23 | 13 | 10 |
| `tests/run_code_model_view.rs` | 15 | 5 | 10 |
| `src/context_pipeline/system.rs` | 11 | 2 | 9 |
| `src/engines/path_index/mod.rs` | 9 | 0 | 9 |
| `src/llm/codec/wire_dump.rs` | 9 | 0 | 9 |
| `src/runtime/exec.rs` | 10 | 1 | 9 |
| `src/runtime/pending_auto_turn.rs` | 14 | 5 | 9 |
| `src/session/estimate.rs` | 9 | 0 | 9 |
| `src/terminal/agent_jobs.rs` | 15 | 6 | 9 |
| `src/tool/output.rs` | 9 | 0 | 9 |
| `src/tools/ask_user.rs` | 9 | 0 | 9 |
| `src/tools/code/render.rs` | 15 | 6 | 9 |
| `src/tools/edit/tests.rs` | 17 | 8 | 9 |
| `src/workspace/filter/exclude.rs` | 9 | 0 | 9 |
| `src/workspace/git.rs` | 13 | 4 | 9 |
| `tests/optional_code_search.rs` | 17 | 8 | 9 |
| `src/engines/progress.rs` | 8 | 0 | 8 |
| `src/engines/session_search/lexical.rs` | 8 | 0 | 8 |
| `src/engines/session_search/parity.rs` | 11 | 3 | 8 |
| `src/session/task_state.rs` | 8 | 0 | 8 |
| `src/telemetry/mod.rs` | 8 | 0 | 8 |
| `src/tool/coords.rs` | 8 | 0 | 8 |
| `src/workspace/filter/walk.rs` | 8 | 0 | 8 |
| `src/config/global_db/seed.rs` | 12 | 5 | 7 |
| `src/engines/session_search/golden.rs` | 7 | 0 | 7 |
| `src/knowledge/spec_tests.rs` | 11 | 4 | 7 |
| `src/knowledge/view.rs` | 7 | 0 | 7 |
| `src/reminder/sources/tests.rs` | 9 | 2 | 7 |
| `src/tools/edit/python_indent.rs` | 8 | 1 | 7 |
| `src/tools/mod.rs` | 7 | 0 | 7 |
| `src/tools/subagent/jobs.rs` | 7 | 0 | 7 |
| `src/tools/websearch.rs` | 9 | 2 | 7 |
| `src/workspace/filter/path_glob.rs` | 7 | 0 | 7 |
| `tests/code_search_ipc.rs` | 11 | 4 | 7 |
| `tests/session_list_empty_filter.rs` | 11 | 4 | 7 |
| `tests/subagent_session_isolation.rs` | 13 | 6 | 7 |
| `src/config/global_db/legacy.rs` | 6 | 0 | 6 |
| `src/config/guides.rs` | 6 | 0 | 6 |
| `src/config/resolved.rs` | 6 | 0 | 6 |
| `src/engines/code_search_engine.rs` | 6 | 0 | 6 |
| `src/engines/session_search/cjk.rs` | 6 | 0 | 6 |
| `src/knowledge/mentions.rs` | 6 | 0 | 6 |
| `src/llm/codec/http.rs` | 12 | 6 | 6 |
| `src/llm/codec/replay.rs` | 6 | 0 | 6 |
| `src/llm/replay_compat.rs` | 11 | 5 | 6 |
| `src/reminder/mentions.rs` | 8 | 2 | 6 |
| `src/reminder/migrate.rs` | 6 | 0 | 6 |
| `src/runtime/phase.rs` | 6 | 0 | 6 |
| `src/session/data/sqlite/schema.rs` | 7 | 1 | 6 |
| `src/session/media.rs` | 6 | 0 | 6 |
| `src/tool/signal.rs` | 6 | 0 | 6 |
| `src/tools/subagent/delete.rs` | 6 | 0 | 6 |
| `src/tools/subagent/hub.rs` | 7 | 1 | 6 |
| `src/workspace/filter/preset.rs` | 6 | 0 | 6 |
| `tests/workspace_api.rs` | 11 | 5 | 6 |
| `src/client_protocol/connection.rs` | 26 | 21 | 5 |
| `src/config/bridge.rs` | 5 | 0 | 5 |
| `src/config/global_db/mod.rs` | 5 | 0 | 5 |
| `src/config/path.rs` | 9 | 4 | 5 |
| `src/config/schema.rs` | 5 | 0 | 5 |
| `src/engines/code_search/atomic.rs` | 6 | 1 | 5 |
| `src/engines/session_search/chunk.rs` | 5 | 0 | 5 |
| `src/engines/session_search/slots.rs` | 5 | 0 | 5 |
| `src/engines/session_search/trimmer.rs` | 5 | 0 | 5 |
| `src/llm/codec/chat_synth.rs` | 6 | 1 | 5 |
| `src/llm/codec/messages_synth.rs` | 8 | 3 | 5 |
| `src/lsp/project_root.rs` | 8 | 3 | 5 |
| `src/lsp/server.rs` | 8 | 3 | 5 |
| `src/reminder/file_tracker.rs` | 5 | 0 | 5 |
| `src/runtime/stream_projection.rs` | 12 | 7 | 5 |
| `src/serve/run.rs` | 6 | 1 | 5 |
| `src/session/media_tokens.rs` | 5 | 0 | 5 |
| `src/tools/code.rs` | 9 | 4 | 5 |
| `src/tools/custom_hub.rs` | 5 | 0 | 5 |
| `src/tools/lsp_feedback.rs` | 5 | 0 | 5 |
| `src/tools/run.rs` | 7 | 2 | 5 |
| `src/tools/subagent/send.rs` | 5 | 0 | 5 |
| `src/tools/subagent/stop.rs` | 5 | 0 | 5 |
| `src/workspace/filter/index_policy.rs` | 6 | 1 | 5 |
| `tests/client_protocol_permission.rs` | 7 | 2 | 5 |
| `tests/startup_order_death_list.rs` | 9 | 4 | 5 |
| `tests/subagent_child_turn_contract.rs` | 8 | 3 | 5 |
| `src/cli/cli.rs` | 4 | 0 | 4 |
| `src/config/global_db/migrate.rs` | 7 | 3 | 4 |
| `src/context_pipeline/budget.rs` | 8 | 4 | 4 |
| `src/context_pipeline/media_budget.rs` | 9 | 5 | 4 |
| `src/engines/code_search/build.rs` | 4 | 0 | 4 |
| `src/engines/code_search/code_tokenize.rs` | 4 | 0 | 4 |
| `src/engines/code_search/facade.rs` | 4 | 0 | 4 |
| `src/engines/code_search/retrieve.rs` | 4 | 0 | 4 |
| `src/engines/code_search_ipc/protocol.rs` | 4 | 0 | 4 |
| `src/engines/session_search/relevance.rs` | 4 | 0 | 4 |
| `src/engines/status_view.rs` | 4 | 0 | 4 |
| `src/knowledge/status.rs` | 5 | 1 | 4 |
| `src/lsp/hub.rs` | 5 | 1 | 4 |
| `src/optional/exa_mcp.rs` | 4 | 0 | 4 |
| `src/permission/messages.rs` | 4 | 0 | 4 |
| `src/runtime/diagnostics.rs` | 4 | 0 | 4 |
| `src/runtime/mod.rs` | 4 | 0 | 4 |
| `src/session/snapshot_paths.rs` | 4 | 0 | 4 |
| `src/terminal/shell.rs` | 4 | 0 | 4 |
| `src/terminal/tee.rs` | 5 | 1 | 4 |
| `src/tool/file_identity.rs` | 4 | 0 | 4 |
| `src/tool/outcome.rs` | 4 | 0 | 4 |
| `src/tools/lsp.rs` | 16 | 12 | 4 |
| `src/workspace/agent_path.rs` | 7 | 3 | 4 |
| `src/workspace/service.rs` | 8 | 4 | 4 |
| `tests/mcp_integration.rs` | 10 | 6 | 4 |
| `tests/optional_p0.rs` | 6 | 2 | 4 |
| `tests/session_turn_entry.rs` | 4 | 0 | 4 |
| `tests/subagent_tool_pipeline.rs` | 7 | 3 | 4 |
| `tests/wire_seq_identity.rs` | 11 | 7 | 4 |
| `src/config/workspace_identity.rs` | 3 | 0 | 3 |
| `src/context_pipeline/compact.rs` | 9 | 6 | 3 |
| `src/engines/code_search/embed.rs` | 3 | 0 | 3 |
| `src/engines/code_search/meta.rs` | 3 | 0 | 3 |
| `src/engines/code_search/stamps.rs` | 3 | 0 | 3 |
| `src/engines/code_search/store.rs` | 3 | 0 | 3 |
| `src/engines/session_search/dense_parity.rs` | 4 | 1 | 3 |
| `src/llm/codec/responses_harden.rs` | 6 | 3 | 3 |
| `src/llm/codec/stream_contract.rs` | 16 | 13 | 3 |
| `src/lsp/conn.rs` | 16 | 13 | 3 |
| `src/lsp/install/npm.rs` | 4 | 1 | 3 |
| `src/optional/mod.rs` | 10 | 7 | 3 |
| `src/permission/engine.rs` | 6 | 3 | 3 |
| `src/permission/presets.rs` | 3 | 0 | 3 |
| `src/platform_knobs.rs` | 3 | 0 | 3 |
| `src/runtime/custom_auto_turn.rs` | 3 | 0 | 3 |
| `src/runtime/subagent_auto_turn.rs` | 4 | 1 | 3 |
| `src/serve/web_dist.rs` | 3 | 0 | 3 |
| `src/session/data/final_immutability_tests.rs` | 8 | 5 | 3 |
| `src/session/data/sqlite/ops.rs` | 4 | 1 | 3 |
| `src/terminal/ansi.rs` | 3 | 0 | 3 |
| `src/tool/code_manifest.rs` | 4 | 1 | 3 |
| `src/tool/write_lock.rs` | 5 | 2 | 3 |
| `src/tools/edit/matcher.rs` | 4 | 1 | 3 |
| `src/tools/edit/planner.rs` | 3 | 0 | 3 |
| `src/tools/subagent/launch.rs` | 3 | 0 | 3 |
| `src/types/result.rs` | 3 | 0 | 3 |
| `src/workspace/file_ops.rs` | 8 | 5 | 3 |
| `src/workspace/filter/dirs.rs` | 3 | 0 | 3 |
| `src/workspace/filter/mod.rs` | 3 | 0 | 3 |
| `src/workspace/sqlite_preview.rs` | 4 | 1 | 3 |
| `src/workspace/symbols.rs` | 3 | 0 | 3 |
| `tests/client_protocol_telemetry.rs` | 3 | 0 | 3 |
| `tests/permission_pipeline.rs` | 14 | 11 | 3 |
| `tests/subagent_send.rs` | 8 | 5 | 3 |
| `src/authority.rs` | 2 | 0 | 2 |
| `src/config/gate/mod.rs` | 2 | 0 | 2 |
| `src/config/log_filter.rs` | 2 | 0 | 2 |
| `src/config/turn_guard.rs` | 2 | 0 | 2 |
| `src/context_pipeline/env.rs` | 3 | 1 | 2 |
| `src/context_pipeline/summary.rs` | 4 | 2 | 2 |
| `src/engines/code_search/fs_notify.rs` | 2 | 0 | 2 |
| `src/engines/code_search/semantic_engine.rs` | 2 | 0 | 2 |
| `src/engines/session_search/derive.rs` | 2 | 0 | 2 |
| `src/engines/session_search/echo.rs` | 2 | 0 | 2 |
| `src/knowledge/fsref.rs` | 2 | 0 | 2 |
| `src/knowledge/index_tests.rs` | 5 | 3 | 2 |
| `src/llm/codec/messages_usage.rs` | 2 | 0 | 2 |
| `src/lsp/install/adapters/pyright.rs` | 2 | 0 | 2 |
| `src/lsp/install/adapters/rust_analyzer.rs` | 2 | 0 | 2 |
| `src/permission/grants.rs` | 4 | 2 | 2 |
| `src/permission/matchers.rs` | 2 | 0 | 2 |
| `src/ptc/boundary.rs` | 2 | 0 | 2 |
| `src/reminder/engine.rs` | 3 | 1 | 2 |
| `src/runtime/observer.rs` | 4 | 2 | 2 |
| `src/session/delete.rs` | 4 | 2 | 2 |
| `src/session/workspace_lock.rs` | 2 | 0 | 2 |
| `src/telemetry/latency.rs` | 2 | 0 | 2 |
| `src/telemetry/log_broadcast.rs` | 2 | 0 | 2 |
| `src/terminal/mod.rs` | 5 | 3 | 2 |
| `src/terminal/pty.rs` | 3 | 1 | 2 |
| `src/tools/bash_safety.rs` | 9 | 7 | 2 |
| `src/tools/kill.rs` | 4 | 2 | 2 |
| `src/tools/subagent/list.rs` | 2 | 0 | 2 |
| `src/tools/wait.rs` | 4 | 2 | 2 |
| `src/types/transcript.rs` | 2 | 0 | 2 |
| `src/version.rs` | 2 | 0 | 2 |
| `src/workspace/filter/path.rs` | 5 | 3 | 2 |
| `tests/agent_loop.rs` | 11 | 9 | 2 |
| `tests/code_tool_runtime.rs` | 4 | 2 | 2 |
| `tests/session_data_bootstrap.rs` | 2 | 0 | 2 |
| `tests/session_data_crud.rs` | 6 | 4 | 2 |
| `tests/session_transcript.rs` | 2 | 0 | 2 |
| `tests/turn_prefix_stability.rs` | 11 | 9 | 2 |
| `src/agent/core.rs` | 2 | 1 | 1 |
| `src/client_protocol/controller/session_lifecycle.rs` | 1 | 0 | 1 |
| `src/client_protocol/output.rs` | 1 | 0 | 1 |
| `src/client_protocol/protocol.rs` | 2 | 1 | 1 |
| `src/client_protocol/status_track.rs` | 5 | 4 | 1 |
| `src/client_protocol/workspace.rs` | 1 | 0 | 1 |
| `src/config/mod.rs` | 1 | 0 | 1 |
| `src/context_pipeline/mod.rs` | 2 | 1 | 1 |
| `src/engines/code_search/chunk.rs` | 1 | 0 | 1 |
| `src/engines/code_search/lexical_primitive.rs` | 1 | 0 | 1 |
| `src/knowledge/corpus.rs` | 1 | 0 | 1 |
| `src/knowledge/reminder.rs` | 1 | 0 | 1 |
| `src/llm/codec/sse.rs` | 7 | 6 | 1 |
| `src/lsp/format.rs` | 6 | 5 | 1 |
| `src/lsp/install/adapters/csharp.rs` | 1 | 0 | 1 |
| `src/lsp/install/adapters/gopls.rs` | 1 | 0 | 1 |
| `src/lsp/install/adapters/typescript.rs` | 2 | 1 | 1 |
| `src/permission/floor.rs` | 2 | 1 | 1 |
| `src/permission/sensitive.rs` | 1 | 0 | 1 |
| `src/ptc/budget.rs` | 1 | 0 | 1 |
| `src/ptc/json.rs` | 1 | 0 | 1 |
| `src/reminder/knowledge_status.rs` | 1 | 0 | 1 |
| `src/runtime/bash_auto_turn.rs` | 4 | 3 | 1 |
| `src/serve/auth.rs` | 1 | 0 | 1 |
| `src/session/data/blob.rs` | 1 | 0 | 1 |
| `src/session/working.rs` | 1 | 0 | 1 |
| `src/tool/snippet.rs` | 1 | 0 | 1 |
| `src/tool/trait_.rs` | 1 | 0 | 1 |
| `src/tools/baseline/mod.rs` | 2 | 1 | 1 |
| `src/tools/file_path.rs` | 1 | 0 | 1 |
| `src/tools/mcp_tool.rs` | 1 | 0 | 1 |
| `src/tools/subagent/spawn_contract.rs` | 2 | 1 | 1 |
| `src/tools/subagent/status.rs` | 1 | 0 | 1 |
| `src/types/error.rs` | 1 | 0 | 1 |
| `src/types/golden_item_fixtures.rs` | 1 | 0 | 1 |
| `src/types/utf8.rs` | 1 | 0 | 1 |
| `src/workspace/citations.rs` | 2 | 1 | 1 |
| `src/workspace/filter/gitignore.rs` | 1 | 0 | 1 |
| `src/workspace/path_sort.rs` | 1 | 0 | 1 |
| `src/workspace/sandbox.rs` | 3 | 2 | 1 |
| `src/workspace/tool_path.rs` | 11 | 10 | 1 |
| `tests/agent_e2e_responses.rs` | 4 | 3 | 1 |
| `tests/compact_failure_calibers.rs` | 5 | 4 | 1 |
| `tests/g3_auth.rs` | 4 | 3 | 1 |
| `tests/pending_messages.rs` | 3 | 2 | 1 |
| `tests/session_data_concurrency.rs` | 3 | 2 | 1 |
| `tests/session_data_fault.rs` | 3 | 2 | 1 |
| `tests/session_data_lifecycle.rs` | 2 | 1 | 1 |
| `tests/session_data_sql_death_list.rs` | 1 | 0 | 1 |
| `tests/session_delete.rs` | 2 | 1 | 1 |
| `tests/workspace_path_consumer.rs` | 5 | 4 | 1 |
| `tests/workspace_path_lap.rs` | 6 | 5 | 1 |
| `tests/workspace_process_contract.rs` | 5 | 4 | 1 |
| **合计（295 个文件）** | 2574 | 677 | 1897 |

注：golden.rs / relevance.rs 已整文件删除（空壳）；parity.rs 恢复「增量=全量重建」3 条并入 protected，故保留。属性计数含 cfg(windows) 等平台测试，与 Linux `--list` 数不完全一致。
