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
