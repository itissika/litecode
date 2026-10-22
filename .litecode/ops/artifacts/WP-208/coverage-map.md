> **已作废（2026-10-22，GM 选 A）**：本文件基于已 revert 的 D1–D5 删除，现以 core-chains.md 为准。

# WP-208 核心链路覆盖地图（lib；3acfe31 → HEAD cb5f33f）

tests/ 未改（D6 已 stash 未提交）。lib：2212 → 811。

| 核心链路 | 3acfe31 | HEAD | 已删 | HEAD 保留的代表测试 |
|---|---|---|---|---|
| run_code/ptc · 序列化 undefined/bigint/boundary | 8 | 4 | 4 | ptc::tests::dropped_undefined_list_is_capped_with_more_count<br>ptc::tests::host_receives_extra_arguments_as_a_json_array<br>ptc::tests::undefined_in_arguments_is_dropped_silently<br>ptc::tests::undefined_in_result_is_stripped_or_nulled_and_reported |
| run_code/ptc · 预算/取消/超时 | 11 | 8 | 3 | ptc::tests::budget_interrupts_infinite_loop<br>ptc::tests::budget_pause_is_not_charged<br>ptc::tests::cancel_grace_records_unfinished<br>ptc::tests::cancel_while_awaiting_host |
| run_code/ptc · 栈与内存封口 | 9 | 9 | 0 | ptc::tests::caught_allocation_failure_is_memory_when_the_stack_watch_is_disarmed<br>ptc::tests::exception_includes_stack<br>ptc::tests::exception_keeps_stack<br>ptc::tests::memory_exhaustion_stays_sealed_when_the_program_catches_it |
| run_code · render 预算/输出 | 24 | 7 | 17 | tools::code::outcome_tests::a_second_javascript_argument_is_rejected_before_the_tool_runs<br>tools::code::outcome_tests::permission_denial_is_counted_as_a_failed_settled_call<br>tools::code::outcome_tests::render_run_output_attaches_by_tool_data_without_leaking_into_content<br>tools::code::render::tests::a_log_that_fills_its_budget_keeps_a_short_result_verbatim |
| run_code · uncaught/失败分类 | 15 | 4 | 11 | ptc::tests::caught_allocation_failure_is_memory_when_the_stack_watch_is_disarmed<br>ptc::tests::second_argument_is_type_error<br>ptc::tests::syntax_error_on_the_second_line_keeps_stack_and_location<br>ptc::tests::uncaught_tool_error_keeps_code |
| tools · read | 26 | 8 | 18 | tools::read::chain_contract_tests::glob_over_limit_notes_truncation_and_read_consumes_path<br>tools::read::chain_contract_tests::outside_workspace_paths_noted<br>tools::read::chain_contract_tests::read_missing_file_is_not_found<br>tools::read::chain_contract_tests::read_page_size_ignores_token_budget |
| tools · grep | 30 | 7 | 23 | tools::grep::message_contract_tests::grep_message_binary_file<br>tools::grep::message_contract_tests::grep_message_glob_empty<br>tools::grep::message_contract_tests::grep_message_path_excluded<br>tools::grep::tests::a_bare_brace_is_invalid_regex |
| tools · glob | 19 | 3 | 16 | tools::glob::tests::cancelled_glob_stops_before_walking<br>tools::glob::tests::matcher_supports_braces<br>tools::glob::tests::parent_dir_pattern_is_invalid_input |
| tools · write（expected_hash/原子/敏感） | 16 | 7 | 9 | tools::write::expected_hash_tests::write_create_only_with_expected_hash_on_missing_file_is_invalid_input<br>tools::write::expected_hash_tests::write_expected_hash_match_returns_file_ref<br>tools::write::expected_hash_tests::write_expected_hash_mismatch_is_stale<br>tools::write::tests::external_all_write_skips_workspace_change |
| tools · edit | 32 | 12 | 20 | tools::edit::matcher::tests::multiline_indent_only_is_structural<br>tools::edit::matcher::tests::multiple_match_message_lists_lines<br>tools::edit::matcher::tests::structural_prefix_and_indent_block_before_apply<br>tools::edit::planner::tests::parse_rejects_replace_all_and_expected_hash |
| tools · apply_edits | 2 | 2 | 0 | tools::apply_edits::tests::schema_rejects_retired_knobs<br>tools::apply_edits::tests::serial_skip_continues_results_same_len |
| tools · run 前台窗/后台 | 7 | 4 | 3 | tools::run::tests::cancel_before_spawn_rejects<br>tools::run::tests::fast_command_returns_exit_code<br>tools::run::tests::validate_rejects_empty_command<br>tools::run::tests::wait_yields_so_cancel_can_run |
| tools · run 锁只到 spawn | 7 | 7 | 0 | tools::run_lock_gate::cancel_before_spawn_releases_when_the_call_ends<br>tools::run_lock_gate::dropped_in_flight_future_releases_and_panic_unwind_releases<br>tools::run_lock_gate::four_parallel_write_runs_finish_in_about_one_second<br>tools::run_lock_gate::locks_release_after_spawn_while_command_still_runs |
| tools · wait/kill | 8 | 6 | 2 | tools::kill::tests::kill_stops_process_and_notes_output<br>tools::kill::tests::permission_is_fixed_allow_without_command_policy<br>tools::kill::tests::unknown_id_is_not_found<br>tools::wait::tests::unknown_id_is_not_found |
| tools · lsp（七工具） | 16 | 6 | 10 | tools::lsp::tests::absolute_outside_root_rejected<br>tools::lsp::tests::all_code_tools_are_code_surface_no_direct_lsp<br>tools::lsp::tests::content_modified_surfaces_warning_on_data_result<br>tools::lsp::tests::directory_rejected |
| tools · webfetch/websearch | 31 | 8 | 23 | tools::webfetch::tests::gate_hard_ssrf_uses_permission_denied_code<br>tools::webfetch::tests::non_http_scheme_is_hard_invalid_input<br>tools::webfetch::tests::private_ip_is_hard_permission_denied<br>tools::webfetch::tests::ssrf_allows_benchmarking_fake_ip_range |
| tools · session_search 溢出/hit=read | 3 | 1 | 2 | tools::session_search::tests::a_result_over_budget_names_the_file_that_holds_the_rest |
| 锁与并发 · resource_gate/write_lock | 13 | 13 | 0 | tool::resource_gate::tests::a_waiting_writer_is_not_skipped_by_a_later_reader<br>tool::resource_gate::tests::cancel_while_queued_does_not_enter<br>tool::resource_gate::tests::distinct_write_keys_do_not_block_each_other<br>tool::resource_gate::tests::file_resource_key_folds_dot_parent_and_separators |
| 执行器 审批/取消/超时 | 21 | 7 | 14 | tool::executor::code_caller_contract_tests::unknown_parameter_is_invalid_input<br>tool::executor::code_caller_contract_tests::write_outside_workspace_reports_path_outside_workspace<br>tool::executor::tests::blocking_tool_is_interrupted_by_pipeline_timeout<br>tool::executor::tests::committed_result_wins_over_tool_timeout |
| SDK fixture 快照 | 2 | 2 | 0 | tool::sdk_snapshot::phase0_run_code_sdk_snapshot<br>tool::sdk_snapshot::sdk_hash_is_stable_for_the_same_tool_set_under_different_cwd_and_permission |
| tool data/error baseline | 2 | 2 | 0 | tools::baseline::phase0_tool_fixtures_lock_data_and_error<br>tools::baseline::phase0_uncovered_scenarios_are_recorded_as_gaps |
| session · 持久化 sqlite（crash/seq/immutability） | 94 | 42 | 52 | session::data::blob::tests::put_is_content_addressed_and_idempotent<br>session::data::final_immutability_tests::a_finalised_row_is_never_rewritten<br>session::data::final_immutability_tests::a_restart_leaves_an_unanswered_call_that_later_history_follows<br>session::data::final_immutability_tests::a_restart_seals_an_in_flight_call_and_appends_its_interrupt_result |
| session · compaction | 41 | 34 | 7 | context_pipeline::budget::tests::compact_retires_the_in_memory_bill_and_the_stored_one<br>context_pipeline::compact::tests::compact_persist_ok_reload_err_keeps_db_truth<br>context_pipeline::compact::tests::compact_request_returns_summary_on_provider_success<br>context_pipeline::compact::tests::low_provider_bill_does_not_compact_a_hot_local_estimate |
| session · manager turn 生命周期 | 45 | 17 | 28 | session::manager::child_session_tests::cancel_during_starting_turn_cancels_on_start<br>session::manager::child_session_tests::cancel_leaves_pending_queue_intact<br>session::manager::child_session_tests::cancel_turn_sync_reports_idle_when_nothing_running<br>session::manager::child_session_tests::compact_lease_atomically_blocks_turn_and_revert |
| provider 流式 · stream_contract/sse | 23 | 20 | 3 | llm::codec::sse::tests::crlf_lines_strip_cr<br>llm::codec::sse::tests::extracts_data_payload<br>llm::codec::sse::tests::leading_bom_is_stripped_once<br>llm::codec::sse::tests::multibyte_char_split_across_chunks_is_preserved |
| provider 流式 · responses | 43 | 8 | 35 | llm::codec::responses::tests::cancel_after_a_delta_seals_incomplete<br>llm::codec::responses::tests::keepalive_between_events_does_not_abort_the_stream<br>llm::codec::responses::tests::non_event_stream_content_type_surfaces_the_proxy_body<br>llm::codec::responses::tests::stream_events_are_forwarded_and_items_come_from_the_terminal |
| provider 流式 · messages | 43 | 11 | 32 | llm::codec::messages::tests::cancel_after_a_delta_seals_incomplete<br>llm::codec::messages::tests::empty_stream_retries_until_a_terminal_arrives<br>llm::codec::messages::tests::image_pdf_and_tool_image_map_to_blocks<br>llm::codec::messages::tests::stream_error_before_items_retries_and_after_items_preserves |
| provider 流式 · chat | 28 | 9 | 19 | llm::codec::chat::tests::encodes_system_and_user_messages_with_stream_usage<br>llm::codec::chat::tests::stream_tool_call_opens_the_item_before_the_arguments_delta<br>llm::codec::chat::tests::stream_usage_lands_on_the_terminal_response<br>llm::codec::chat::tests::stream_whitespace_only_content_leaves_no_message_item |
| context pipeline 预算/阈值/keep_recent | 32 | 16 | 16 | context_pipeline::budget::tests::compact_retires_the_in_memory_bill_and_the_stored_one<br>context_pipeline::budget::tests::low_provider_bill_stays_under_threshold_when_local_estimate_is_over<br>context_pipeline::keep_recent::tests::assistant_segment_includes_function_calls_with_reasoning<br>context_pipeline::keep_recent::tests::assistant_segment_not_split_between_reasoning_and_message |
| 权限 · preset/floor/subagent | 24 | 24 | 0 | permission::engine::tests::a_tool_named_run_code_keeps_safe_deny<br>permission::engine::tests::all_and_removed_rules_allow<br>permission::engine::tests::reserved_names_keep_the_safe_binding<br>permission::engine::tests::safe_uses_the_live_rules |
| 路径安全 · 越界/敏感/canonicalize | 96 | 32 | 64 | config::path::tests::canon_abs_resolves_symlink<br>config::path::tests::strip_uppercases_drive_letter<br>config::path::tests::strip_verbatim_drive_and_unc<br>engines::code_search::enclosing::tests::top_level_line_outside_fn_may_be_empty_or_mod_only |
| LSP · 冷启动/导航/路径 | 60 | 19 | 41 | lsp::conn::tests::content_modified_semantic_tokens_is_cancelled_once<br>lsp::conn::tests::unversioned_publish_after_change_is_silence_not_stale_errors<br>lsp::conn::tests::wait_ignores_stale_publish_and_keeps_waiting_for_current<br>lsp::conn::tests::wait_returns_current_clean_cache_without_blocking |
| terminal · agent_jobs | 34 | 15 | 19 | terminal::agent_jobs::tests::detached_read_continues_after_nested_cancel<br>terminal::agent_jobs::tests::foreground_cancel_kills_and_skips_mailbox<br>terminal::agent_jobs::tests::foreground_timeout_promotes_without_killing<br>terminal::ansi::tests::removes_csi_color_codes |
| code_search 索引 | 126 | 26 | 100 | engines::code_search::atomic::tests::append_syncs_parent_dir_only_when_creating_the_file<br>engines::code_search::build::tests::code_index_corpus_follows_extensions_not_search_exclude<br>engines::code_search::cool_tests::pending_blocks_cool_drop<br>engines::code_search::cool_tests::tick_drains_queue_without_pending_hint |

## 已删但属于核心链路的测试（按链路）

### run_code/ptc · 序列化 undefined/bigint/boundary（4）
- `ptc::boundary::tests::json_bytes_counts_a_deep_array_on_the_heap`
- `ptc::boundary::tests::json_bytes_matches_serde_for_shallow_values`
- `ptc::json::tests::shallow_clone_eq_and_debug_match_serde`
- `ptc::tests::returns_json_values`

### run_code/ptc · 预算/取消/超时（3）
- `ptc::budget::tests::paused_wait_is_not_charged`
- `ptc::engine::settle_tests::cancel_during_drain_keeps_the_value`
- `ptc::engine::settle_tests::timeout_during_drain_keeps_the_returned_value`

### run_code · render 预算/输出（17）
- `tools::code::outcome_tests::a_finished_call_keeps_the_host_outcome`
- `tools::code::outcome_tests::ledger_keeps_a_priority_row_past_the_head_and_marks_a_hidden_session_read`
- `tools::code::outcome_tests::omitted_session_read_marks_the_ledger_and_states_the_count`
- `tools::code::outcome_tests::unfinished_call_stays_unsettled_even_when_the_host_later_records_ok`
- `tools::code::outcome_tests::unknown_host_outcome_is_unsettled`
- `tools::code::outcome_tests::wont_start_data_is_minimal_without_by_tool`
- `tools::code::render::tests::default_view_stays_inside_the_budget_without_legacy_sections`
- `tools::code::render::tests::failure_header_counts_and_marks_unsettled`
- `tools::code::render::tests::failure_keeps_partial_result`
- `tools::code::render::tests::human_data_includes_unsettled_and_failure_kind`
- `tools::code::render::tests::human_data_matches_head_counts_and_stays_out_of_content`
- `tools::code::render::tests::human_data_wont_start_is_minimal`
- `tools::code::render::tests::large_result_keeps_the_tail_and_states_the_omission`
- `tools::code::render::tests::session_read_mark_lands_on_the_head`
- `tools::code::render::tests::small_string_is_verbatim_and_has_no_json_copy`
- `tools::code::render::tests::success_header_counts_failures_from_call_states`
- `tools::code::render::tests::syntax_failure_puts_line_and_cause_on_the_head`

### run_code · uncaught/失败分类（11）
- `ptc::engine::settle_tests::other_drain_failures_replace_success`
- `ptc::tests::console_formats_errors`
- `ptc::tests::host_error_finishes_the_call`
- `ptc::tests::syntax_error`
- `ptc::tests::tool_error_is_catchable_and_instanceof`
- `ptc::tests::type_error_from_host_is_catchable`
- `tools::code::render::tests::failure_header_counts_and_marks_unsettled`
- `tools::code::render::tests::failure_keeps_partial_result`
- `tools::code::render::tests::human_data_includes_unsettled_and_failure_kind`
- `tools::code::render::tests::success_header_counts_failures_from_call_states`
- `tools::code::render::tests::syntax_failure_puts_line_and_cause_on_the_head`

### tools · read（18）
- `tools::read::chain_contract_tests::chain_glob_read`
- `tools::read::chain_contract_tests::chain_grep_read`
- `tools::read::chain_contract_tests::chain_read_edit`
- `tools::read::tests::chunked_read_returns_the_whole_file`
- `tools::read::tests::data_schema_accepts_live_result`
- `tools::read::tests::test_read_basic`
- `tools::read::tests::test_read_binary_detection`
- `tools::read::tests::test_read_default_limit_pages`
- `tools::read::tests::test_read_empty_file_past_end_resolves_empty`
- `tools::read::tests::test_read_image_is_unsupported`
- `tools::read::tests::test_read_long_line_is_capped_in_text`
- `tools::read::tests::test_read_page_ignores_token_count`
- `tools::read::tests::test_read_preserves_trailing_spaces_on_last_line`
- `tools::read::tests::test_read_schema_omits_retired_fields`
- `tools::read::tests::test_read_start_line_past_end_resolves_empty`
- `tools::read::tests::test_read_strips_utf8_bom_from_display`
- `tools::read::tests::test_read_utf16_error_is_explicit`
- `tools::read::tests::test_validate_input`

### tools · grep（23）
- `tools::grep::message_contract_tests::grep_literal_searches_metacharacters`
- `tools::grep::message_contract_tests::grep_message_corpus_empty`
- `tools::grep::message_contract_tests::grep_message_no_match`
- `tools::grep::tests::a_single_file_with_many_hits_still_bounds`
- `tools::grep::tests::a_valid_regex_still_distinguishes_matches`
- `tools::grep::tests::an_unclosed_class_is_not_searched_as_literal_text`
- `tools::grep::tests::data_schema_accepts_structured_hits`
- `tools::grep::tests::empty_workspace_reports_no_files`
- `tools::grep::tests::examine_cap_bounds_hits_in_message`
- `tools::grep::tests::excluded_tree_stays_hidden_without_dialect_knob`
- `tools::grep::tests::grep_does_not_consult_the_path_index`
- `tools::grep::tests::ignore_case_finds_mixed_case`
- `tools::grep::tests::invalid_limit_is_rejected`
- `tools::grep::tests::match_text_is_bounded`
- `tools::grep::tests::multiple_hits_in_one_file_render_each_occurrence`
- `tools::grep::tests::naming_an_excluded_tree_explains_in_message`
- `tools::grep::tests::pattern_is_required_and_must_be_nonempty`
- `tools::grep::tests::retired_cursor_is_absent_from_schema`
- `tools::grep::tests::schema_exposes_simple_contract`
- `tools::grep::tests::unicode_matches_render_intact`
- `tools::grep::tests::virtual_session_grep_miss_stays_a_plain_miss`
- `tools::grep::tests::virtual_session_grep_still_answers`
- `tools::grep::tests::wide_search_never_writes_a_spill_file`

### tools · glob（16）
- `tools::glob::tests::basename_glob_recurses`
- `tools::glob::tests::data_schema_accepts_structured_results`
- `tools::glob::tests::default_filters_hide_gitignored`
- `tools::glob::tests::empty_glob_sets_message`
- `tools::glob::tests::explicit_session_dir_glob_lists_every_session`
- `tools::glob::tests::file_glob_skips_search_exclude_keeps_hidden`
- `tools::glob::tests::glob_lists_binary_png`
- `tools::glob::tests::glob_match_view_is_depth_grouped_not_mtime`
- `tools::glob::tests::glob_reaches_the_knowledge_root_and_refuses_the_rest`
- `tools::glob::tests::hits_sort_by_depth_then_parent_then_name`
- `tools::glob::tests::path_into_excluded_dir_explains_in_message`
- `tools::glob::tests::repeated_path_prefix_is_not_stripped`
- `tools::glob::tests::schema_drops_retired_knobs`
- `tools::glob::tests::structured_result_lists_paths`
- `tools::glob::tests::truncates_with_message`
- `tools::glob::tests::virtual_sessions_list_only_when_path_is_explicit`

### tools · write（expected_hash/原子/敏感）（9）
- `tools::write::expected_hash_tests::write_expected_hash_on_missing_file_does_not_create`
- `tools::write::tests::ide_write_emits_workspace_change_for_inner_path`
- `tools::write::tests::test_is_destructive`
- `tools::write::tests::test_validate_input`
- `tools::write::tests::test_write_create_new`
- `tools::write::tests::test_write_create_only_refuses_overwrite`
- `tools::write::tests::test_write_creates_parent_dirs`
- `tools::write::tests::test_write_update_existing`
- `tools::write::tests::virtual_session_path_is_read_only`

### tools · edit（20）
- `tools::edit::matcher::tests::near_typo_is_no_match`
- `tools::edit::python_indent::exact_match_still_applies`
- `tools::edit::python_indent::exact_old_wrong_new_indent_applies_and_breaks_python`
- `tools::edit::python_indent::fully_dedented_old_is_whitespace_only_reject`
- `tools::edit::python_indent::resolve_one_smoke`
- `tools::edit::python_indent::underindented_old_substring_matches_deeper_line`
- `tools::edit::tests::edit_declares_a_thirty_second_timeout`
- `tools::edit::tests::edit_tool_does_not_equate_curly_quotes`
- `tools::edit::tests::edit_tool_is_destructive_on_empty_new_string`
- `tools::edit::tests::edit_tool_roundtrips_bom_and_matches_first_line`
- `tools::edit::tests::edit_tool_schema_accepts_flat_shape_rejects_old`
- `tools::edit::tests::edit_tool_wire_multiple_matches`
- `tools::edit::tests::edit_tool_wire_noop_and_empty_file`
- `tools::edit::tests::edit_tool_wire_view_prefixes_and_indent`
- `tools::edit::tests::empty_new_string_deletes_old`
- `tools::edit::tests::failure_message_tests::edit_failure_messages_are_stable`
- `tools::edit::tests::failure_message_tests::edit_failure_resolves_with_message`
- `tools::edit::tests::single_success_reports_edit_result`
- `tools::edit::tests::unmatched_reports_no_match_without_applying`
- `tools::edit::tests::virtual_session_path_is_read_only`

### tools · run 前台窗/后台（3）
- `tools::run::tests::effects_and_safe_policy_match_readonly_dial`
- `tools::run::tests::long_command_returns_null_exit_and_keeps_job`
- `tools::run::tests::surface_schema_and_command_shape`

### tools · wait/kill（2）
- `tools::kill::tests::surface_schema_and_admission`
- `tools::wait::tests::surface_schema_and_clamp`

### tools · lsp（七工具）（10）
- `tools::lsp::tests::character_out_of_range_is_hard_error`
- `tools::lsp::tests::character_utf16_multibyte`
- `tools::lsp::tests::code_tool_names_are_snake_case_seven`
- `tools::lsp::tests::finish_hover_shape`
- `tools::lsp::tests::finish_locations_empty_resolves_with_empty_array`
- `tools::lsp::tests::finish_symbols_workspace_paths_relative`
- `tools::lsp::tests::per_server_gate_matches_only_failed_program`
- `tools::lsp::tests::position_schema_requires_file_path_line_character`
- `tools::lsp::tests::relative_path_resolves_under_root`
- `tools::lsp::tests::workspace_symbol_schema_is_query_only`

### tools · webfetch/websearch（23）
- `tools::webfetch::tests::client_for_literal_ip_needs_no_pin`
- `tools::webfetch::tests::client_pins_hostname_to_fake_ip`
- `tools::webfetch::tests::client_pins_hostname_to_validated_public_ips`
- `tools::webfetch::tests::empty_url_is_hard_invalid_input`
- `tools::webfetch::tests::html_to_markdown_preserves_heading`
- `tools::webfetch::tests::not_warmed_is_hard_unavailable`
- `tools::webfetch::tests::ok_data_helper_still_resolves`
- `tools::webfetch::tests::resolve_allows_literal_fake_ip_through_gate`
- `tools::webfetch::tests::resolve_returns_literal_public_ips`
- `tools::webfetch::tests::soft_fail_shape_has_ok_false_without_error_level`
- `tools::webfetch::tests::ssrf_allows_hostname_resolved_to_fake_ip`
- `tools::webfetch::tests::ssrf_allows_public_http_urls`
- `tools::webfetch::tests::ssrf_rejects_non_http_schemes`
- `tools::webfetch::tests::ssrf_shared_cgnat_still_non_public`
- `tools::webfetch::tests::strip_html_tags_removes_markup`
- `tools::webfetch::tests::success_shape_has_ok_true_body`
- `tools::webfetch::tests::surface_is_code`
- `tools::websearch::tests::client_without_endpoint_is_not_configured`
- `tools::websearch::tests::missing_query_is_error`
- `tools::websearch::tests::not_warmed_surfaces_configuration_hint`
- `tools::websearch::tests::soft_fail_shape_resolves_ok_false`
- `tools::websearch::tests::success_shape_has_text`
- `tools::websearch::tests::surface_is_code`

### tools · session_search 溢出/hit=read（2）
- `engines::session_search::tests::agent_view_spills_the_remainder_into_a_named_file`
- `tools::session_search::tests::search_hit_line_matches_virtual_read`

### 执行器 审批/取消/超时（14）
- `tool::executor::tests::blob_ref_image_becomes_input_image_data_url`
- `tool::executor::tests::cancellable_tools_opt_in_and_the_default_is_off`
- `tool::executor::tests::custom_shaped_schema_rejects_missing_required`
- `tool::executor::tests::custom_shaped_schema_rejects_wrong_type`
- `tool::executor::tests::direct_tool_wrong_parameter_type_is_unified_error`
- `tool::executor::tests::invalid_json_arguments_are_not_swallowed`
- `tool::executor::tests::local_file_after_finalize_encodes_as_input_image`
- `tool::executor::tests::mailbox_stays_pending_for_the_next_seam`
- `tool::executor::tests::missing_blob_returns_err`
- `tool::executor::tests::partition_splits_same_path_read_and_write`
- `tool::executor::tests::shell_exit_does_not_wrap_reminder`
- `tool::executor::tests::unknown_parameters_warn_and_still_run`
- `tool::executor::tests::unmaterialized_local_file_returns_err`
- `tool::executor::tests::video_becomes_input_file`

### session · 持久化 sqlite（crash/seq/immutability）（52）
- `session::data::sqlite::ops::tests::persist_receipt_deleted_accepts_missing_session_row`
- `session::data::sqlite::ops::tests::persist_receipt_deleted_rejects_surviving_session_row`
- `session::data::sqlite::ops::tests::persist_receipt_fails_when_session_row_missing`
- `session::data::sqlite::ops::tests::persist_receipt_updates_revision_when_session_exists`
- `session::data::sqlite::schema::tests::existing_session_db_adds_responsibility_without_losing_sessions`
- `session::data::sqlite::schema::tests::existing_session_db_drops_retired_content_outbox`
- `session::data::sqlite::schema::tests::fresh_db_builds_base_tables_before_fts`
- `session::data::sqlite::schema::tests::half_old_sessions_schema_fails_closed`
- `session::data::sqlite::session::tests::a_later_call_may_reuse_a_provider_id`
- `session::data::sqlite::session::tests::a_new_call_may_reuse_an_id_removed_by_truncate`
- `session::data::sqlite::session::tests::a_pending_tool_result_is_a_new_append_even_when_an_older_result_matches`
- `session::data::sqlite::session::tests::a_streaming_row_cannot_be_re_pointed_at_another_item`
- `session::data::sqlite::session::tests::assistant_only_delta_does_not_wipe_last_message`
- `session::data::sqlite::session::tests::begin_stream_item_opens_in_flight_without_payload_status`
- `session::data::sqlite::session::tests::begin_stream_item_requires_a_turn_id`
- `session::data::sqlite::session::tests::commit_delta_discards_when_turn_view_is_shorter`
- `session::data::sqlite::session::tests::commit_delta_does_not_discard_when_empty_assistant_is_off_surface`
- `session::data::sqlite::session::tests::commit_delta_does_not_insert_orphan_output`
- `session::data::sqlite::session::tests::commit_of_unchanged_in_progress_payload_still_seals_the_row`
- `session::data::sqlite::session::tests::commit_orphan_cleanup_leaves_memory_untouched_when_tx_fails`
- `session::data::sqlite::session::tests::context_meter_migrates_cumulative_columns`
- `session::data::sqlite::session::tests::context_meter_roundtrip`
- `session::data::sqlite::session::tests::derive_messages_after_replace_matches_surface_not_log_tail`
- `session::data::sqlite::session::tests::duplicate_hi_after_assistant_is_two_user_rows`
- `session::data::sqlite::session::tests::empty_messages_leftover_is_dropped`
- `session::data::sqlite::session::tests::fresh_db_has_transcript_items_no_messages_table`
- `session::data::sqlite::session::tests::half_old_sessions_schema_fails_closed`
- `session::data::sqlite::session::tests::in_progress_assistant_does_not_fill_last_assistant`
- `session::data::sqlite::session::tests::incremental_surface_matches_fold_after_compact_and_append`
- `session::data::sqlite::session::tests::job_exit_roundtrips_into_agent_working_set`
- `session::data::sqlite::session::tests::load_events_roundtrip_append_origin_is_contiguous_seq`
- `session::data::sqlite::session::tests::load_working_set_matches_transcript_and_skips_empty_assistant`
- `session::data::sqlite::session::tests::messages_with_rows_fails_closed`
- `session::data::sqlite::session::tests::pad_unanswered_calls_inserts_after_call_block`
- `session::data::sqlite::session::tests::pad_unanswered_calls_keeps_hanging_pad_before_later_answered_batch`
- `session::data::sqlite::session::tests::pad_unanswered_calls_skips_already_answered`
- `session::data::sqlite::session::tests::persist_item_then_commit_does_not_duplicate_assistant`
- `session::data::sqlite::session::tests::plan_execute_roundtrips_into_agent_working_set`
- `session::data::sqlite::session::tests::plan_reminder_roundtrips_into_agent_working_set`
- `session::data::sqlite::session::tests::provider_usage_column_migrates_onto_existing_transcript`
- `session::data::sqlite::session::tests::retired_model_reminder_stays_off_the_spine`
- `session::data::sqlite::session::tests::revert_preview_is_text_not_json_brace`
- `session::data::sqlite::session::tests::revert_removing_last_checkpoint_resets_pointers`
- `session::data::sqlite::session::tests::seal_in_progress_items_marks_incomplete_and_final`
- `session::data::sqlite::session::tests::seal_stream_item_is_refused_on_a_settled_row`
- `session::data::sqlite::session::tests::snip_stale_results_removes_orphan_outputs`
- `session::data::sqlite::session::tests::task_state_roundtrips_plan_revision`
- `session::data::sqlite::session::tests::transcript_items_missing_envelope_columns_fails_closed`
- `session::data::sqlite::session::tests::truncated_tool_result_cjk_boundary`
- `session::data::sqlite::session::tests::turn_end_reason_schema_covers_every_variant`
- `session::data::sqlite::session::tests::user_anchor_query_without_role_column`
- `session::data::sqlite::session::tests::user_anchor_window_is_a_cursor_page`

### session · compaction（7）
- `context_pipeline::budget::tests::manual_compact_gate_is_inclusive_at_thirty_percent`
- `context_pipeline::compact::tests::compact_request_disables_thinking_and_uses_caller_cap`
- `context_pipeline::compact::tests::compact_request_observes_cancel_while_waiting`
- `context_pipeline::compact::tests::compact_request_times_out_on_silent_provider`
- `context_pipeline::compact::tests::compact_summary_drops_reasoning_items`
- `context_pipeline::compact::tests::compact_summary_empty_when_only_reasoning`
- `session::data::sqlite::session::tests::incremental_surface_matches_fold_after_compact_and_append`

### session · manager turn 生命周期（28）
- `session::manager::child_session_tests::activity_snapshot_reports_running_turn_and_ids`
- `session::manager::child_session_tests::begin_turn_rejects_second_while_running`
- `session::manager::child_session_tests::channel_observer_never_tags_parent_session_id`
- `session::manager::child_session_tests::child_counts_tracks_subtree_running_and_idle`
- `session::manager::child_session_tests::child_session_depth_is_parent_plus_one`
- `session::manager::child_session_tests::child_turn_lifecycle_is_broadcast`
- `session::manager::child_session_tests::finish_turn_clears_running_immediately_and_allows_rebegin`
- `session::manager::child_session_tests::open_child_and_remove_parent_cascades_in_memory`
- `session::manager::child_session_tests::orphan_child_sessions_are_cleaned`
- `session::manager::child_session_tests::remove_parent_releases_running_child_turn_guard_and_cancels_child`
- `session::manager::child_session_tests::remove_running_session_releases_guard`
- `session::manager::child_session_tests::remove_session_releases_starting_reservation_guard`
- `session::manager::child_session_tests::reserve_turn_keeps_a_failed_reconnect_retryable`
- `session::manager::child_session_tests::revert_during_running_turn_waits_for_finish_and_holds_exclusive_lease`
- `session::manager::child_session_tests::revert_during_starting_turn_releases_reservation`
- `session::manager::child_session_tests::session_status_aggregates_descendant_turns_and_stopping`
- `session::manager::child_session_tests::start_turn_fanout_spawn_failure_rolls_back_running_and_turn_guard`
- `session::manager::child_session_tests::start_turn_fanout_survives_caller_runtime_drop`
- `session::manager::child_session_tests::turn_reservation_closes_pre_spawn_idle_window`
- `session::manager::pending_message_tests::a_failed_revert_keeps_the_queue`
- `session::manager::pending_message_tests::enqueue_remove_and_claim_never_touch_the_log`
- `session::manager::pending_message_tests::reserve_and_claim_is_a_single_boundary`
- `session::manager::plan_settle_tests::clears_missing_plan_then_persists_and_publishes`
- `session::manager::plan_settle_tests::keeps_active_plan_when_file_exists`
- `session::manager::request_origin_tests::a_session_without_records_reports_no_origin`
- `session::manager::request_origin_tests::failed_model_change_rolls_back_state_and_event_together`
- `session::manager::request_origin_tests::request_origin_round_trips_as_a_control_plane_row`
- `session::manager::turn_step_tests::maps_text_delta_and_reasoning_added`

### provider 流式 · stream_contract/sse（3）
- `llm::codec::sse::tests::oversized_line_is_a_hard_error`
- `llm::codec::sse::tests::skips_noise`
- `llm::codec::sse::tests::trailing_line_without_newline_is_kept_for_finish`

### provider 流式 · responses（35）
- `llm::codec::responses::tests::a_tier_whose_literal_is_off_does_not_force_replay`
- `llm::codec::responses::tests::ark_uses_the_thinking_switch_and_store_extra_body`
- `llm::codec::responses::tests::cancel_before_any_event_is_canceled`
- `llm::codec::responses::tests::chat_history_does_not_replay_synthetic_responses_ids`
- `llm::codec::responses::tests::compact_summary_does_not_send_an_empty_id`
- `llm::codec::responses::tests::empty_reasoning_keeps_its_slot_without_invented_text`
- `llm::codec::responses::tests::foreign_item_ids_do_not_break_tool_pairing`
- `llm::codec::responses::tests::foreign_reasoning_id_is_stripped_and_missing_text_stays_missing`
- `llm::codec::responses::tests::foreign_summary_reaches_text_replay_responses`
- `llm::codec::responses::tests::http_errors_name_the_catalog_provider`
- `llm::codec::responses::tests::items_without_ids_are_never_collapsed`
- `llm::codec::responses::tests::json_output_is_intent_gated_by_capability`
- `llm::codec::responses::tests::keepalive_is_skipped_and_other_unknown_types_still_fail`
- `llm::codec::responses::tests::media_parts_use_vendor_documented_names`
- `llm::codec::responses::tests::model_without_tiers_never_sends_reasoning`
- `llm::codec::responses::tests::nonconforming_tool_call_events_parse_as_function_calls`
- `llm::codec::responses::tests::output_only_status_never_reaches_the_wire`
- `llm::codec::responses::tests::plain_model_sends_the_authority_shape`
- `llm::codec::responses::tests::reasoning_summary_rides_the_declared_literal`
- `llm::codec::responses::tests::repeated_item_ids_are_not_rewritten_by_the_codec`
- `llm::codec::responses::tests::replay_does_not_invent_reasoning_the_model_omitted`
- `llm::codec::responses::tests::request_headers_come_from_the_catalog`
- `llm::codec::responses::tests::session_header_falls_back_to_global`
- `llm::codec::responses::tests::summary_replay_sends_own_id_and_summary_text`
- `llm::codec::responses::tests::thinking_off_does_not_send_temperature`
- `llm::codec::responses::tests::thinking_off_omits_the_whole_control`
- `llm::codec::responses::tests::thinking_off_sends_the_declared_off_literal`
- `llm::codec::responses::tests::tiers_map_each_platform_level_to_its_own_literal`
- `llm::codec::responses::tests::tools_use_the_responses_function_shape`
- `llm::codec::responses::tests::transport_disconnect_returns_partial_items_with_an_explicit_error`
- `llm::codec::responses::tests::truncated_sse_after_items_is_terminal_and_keeps_partials`
- `llm::codec::responses::tests::undeclared_dialect_replays_only_its_own_ciphertext`
- `llm::codec::responses::tests::usage_patch_is_applied_before_deserialization`
- `llm::codec::responses_harden::tests::deepseek_patch_also_fills_total_and_effort`
- `llm::codec::responses_harden::tests::shell_fill_completes_early_events`

### provider 流式 · messages（32）
- `llm::codec::messages::tests::a_missing_tool_result_is_repaired`
- `llm::codec::messages::tests::a_model_without_an_off_literal_stays_adaptive`
- `llm::codec::messages::tests::a_reminder_after_a_tool_turn_keeps_its_thinking`
- `llm::codec::messages::tests::a_reminder_stays_behind_the_tool_result`
- `llm::codec::messages::tests::a_trailing_reminder_keeps_the_open_tool_turns_thinking`
- `llm::codec::messages::tests::a_user_image_drops_only_the_turn_that_saw_it`
- `llm::codec::messages::tests::an_orphan_tool_result_becomes_text`
- `llm::codec::messages::tests::cache_breakpoints_skip_thinking_and_stay_within_three`
- `llm::codec::messages::tests::compaction_keeps_thinking_signed_against_the_summary`
- `llm::codec::messages::tests::disconnect_after_items_keeps_the_partial`
- `llm::codec::messages::tests::http_400_names_the_upstream_error_and_does_not_retry`
- `llm::codec::messages::tests::later_turn_extends_the_previous_messages_prefix`
- `llm::codec::messages::tests::off_shapes_follow_the_catalog_literal`
- `llm::codec::messages::tests::omitted_thinking_replays_an_empty_body`
- `llm::codec::messages::tests::overloaded_status_is_retried`
- `llm::codec::messages::tests::redacted_thinking_replays_without_the_storage_prefix`
- `llm::codec::messages::tests::reserved_messages_keys_are_refused_at_load`
- `llm::codec::messages::tests::signed_thinking_stays_when_the_transcript_opens_on_a_summary`
- `llm::codec::messages::tests::stream_round_trip_keeps_the_signature_and_sends_the_version_header`
- `llm::codec::messages::tests::switching_away_and_back_keeps_each_haiku_signature`
- `llm::codec::messages::tests::text_turn_sends_adaptive_thinking_and_no_sampling`
- `llm::codec::messages::tests::thinking_and_tool_signatures_round_trip_inside_the_open_turn`
- `llm::codec::messages::tests::thinking_binding_header_keeps_other_betas`
- `llm::codec::messages::tests::thinking_from_a_closed_turn_stays`
- `llm::codec::messages::tests::thinking_off_drops_signed_history_before_encoding`
- `llm::codec::messages::tests::two_tool_shots_then_a_later_turn_keeps_signatures_after_the_notes`
- `llm::codec::messages::tests::undeclared_media_degrades_to_text`
- `llm::codec::messages::tests::zero_max_tokens_uses_the_model_cap`
- `llm::codec::messages_synth::tests::citations_delta_is_ignored`
- `llm::codec::messages_synth::tests::max_tokens_and_refusal_mark_the_response_incomplete`
- `llm::codec::messages_synth::tests::omitted_thinking_keeps_an_empty_body_and_the_signature`
- `llm::codec::messages_synth::tests::thinking_text_and_tool_round_trip_with_the_signature`

### provider 流式 · chat（19）
- `llm::codec::chat::tests::chat_usage_maps_vendor_shapes`
- `llm::codec::chat::tests::extra_body_is_merged_last`
- `llm::codec::chat::tests::headers_come_from_the_catalog_and_identification_is_litecode`
- `llm::codec::chat::tests::http_errors_name_the_provider_and_the_protocol`
- `llm::codec::chat::tests::idless_items_remain_distinct_even_with_the_same_call_id`
- `llm::codec::chat::tests::json_output_is_intent_gated_by_capability`
- `llm::codec::chat::tests::model_flags_drive_optional_fields`
- `llm::codec::chat::tests::ported_claude_text_reaches_chat_after_signature_strip`
- `llm::codec::chat::tests::ported_foreign_summary_reaches_chat_reasoning_content`
- `llm::codec::chat::tests::reasoning_effort_uses_the_models_own_literals`
- `llm::codec::chat::tests::recorded_reasoning_is_replayed_as_reasoning_content`
- `llm::codec::chat::tests::repeated_assistant_item_ids_preserve_both_messages`
- `llm::codec::chat::tests::replay_key_is_written_for_every_assistant_turn_while_tools_are_present`
- `llm::codec::chat::tests::session_header_is_global_without_a_session`
- `llm::codec::chat::tests::stream_text_only_becomes_an_assistant_message`
- `llm::codec::chat::tests::thinking_off_omits_reasoning_effort`
- `llm::codec::chat::tests::thinking_off_sends_the_declared_off_literal`
- `llm::codec::chat_synth::tests::whitespace_between_visible_tokens_stays_in_the_message`
- `llm::codec::chat_synth::tests::whitespace_only_content_never_births_a_message`

### context pipeline 预算/阈值/keep_recent（16）
- `context_pipeline::budget::tests::manual_compact_gate_is_inclusive_at_thirty_percent`
- `context_pipeline::budget::tests::provider_bill_covers_its_prefix_and_estimates_only_the_suffix`
- `context_pipeline::budget::tests::rewritten_view_does_not_reuse_a_provider_bill`
- `context_pipeline::budget::tests::ring_occupancy_prefers_provider_prompt`
- `context_pipeline::budget::tests::token_count_does_not_apply_a_separate_media_slice`
- `context_pipeline::budget::tests::token_count_prefers_provider_or_falls_back_to_estimate`
- `context_pipeline::keep_recent::tests::default_keep_recent_caps_at_20k`
- `context_pipeline::keep_recent::tests::fresh_prompt_without_leading_summary`
- `context_pipeline::keep_recent::tests::update_prompt_when_leading_summary`
- `session::estimate::tests::breakdown_empty_is_zero`
- `session::estimate::tests::cl100k_gold_sample_hello_world`
- `session::estimate::tests::empty_transcript_is_zero`
- `session::estimate::tests::estimate_grows_with_text`
- `session::estimate::tests::media_adds_tokens_beyond_text_only`
- `session::estimate::tests::prompt_overhead_fills_system_and_schema`
- `session::estimate::tests::text_token_count_handles_utf8_and_grows_monotonically`

### 路径安全 · 越界/敏感/canonicalize（64）
- `config::path::tests::canon_abs_lossy_on_missing`
- `config::path::tests::canon_abs_round_trips_tempdir`
- `config::path::tests::canon_join_nonexistent_under_root`
- `config::path::tests::is_under_tempdir_file`
- `workspace::filter::corpus_note_tests::search_corpus_next_step_names_fields`
- `workspace::filter::corpus_note_tests::search_corpus_note_cites_excludes_file_and_caps_lists`
- `workspace::filter::corpus_note_tests::search_corpus_note_omits_when_no_ignore`
- `workspace::filter::dirs::tests::discovery_includes_git_and_node_modules`
- `workspace::filter::dirs::tests::product_internal_helpers`
- `workspace::filter::dirs::tests::snapshot_composes_discovery_product_and_only`
- `workspace::filter::exclude::tests::bare_directory_name_folds_children`
- `workspace::filter::exclude::tests::classify_segment_and_complex`
- `workspace::filter::exclude::tests::deep_node_modules_and_bower_segments`
- `workspace::filter::exclude::tests::files_exclude_hits_git_and_ds_store`
- `workspace::filter::exclude::tests::search_exclude_alone_misses_git`
- `workspace::filter::exclude::tests::text_search_inherits_files_and_search`
- `workspace::filter::exclude::tests::watcher_defaults_skip_product_internal_index`
- `workspace::filter::exclude::tests::watcher_excludes_git_objects_not_source`
- `workspace::filter::gitignore::tests::search_honors_gitignore_when_switch_on`
- `workspace::filter::index_policy::tests::gitignore_target_does_not_queue_create`
- `workspace::filter::index_policy::tests::queue_gate_uses_code_index_face`
- `workspace::filter::index_policy::tests::scannable_is_not_a_language_list`
- `workspace::filter::index_policy::tests::search_exclude_does_not_gate_code_index_queue`
- `workspace::filter::path::tests::cheap_rel_under_strips_without_canon`
- `workspace::filter::path::tests::rel_path_ctx_matches_rel_path_under`
- `workspace::filter::path_glob::tests::backslash_normalized`
- `workspace::filter::path_glob::tests::basename_only_star`
- `workspace::filter::path_glob::tests::brace_expansion_ts_tsx`
- `workspace::filter::path_glob::tests::comma_inside_braces_not_split`
- `workspace::filter::path_glob::tests::comma_separated_patterns`
- `workspace::filter::path_glob::tests::split_glob_positive_and_negation`
- `workspace::filter::path_glob::tests::unicode_path_safe`
- `workspace::filter::preset::tests::browse_search_split_explorer_reads_own_switch`
- `workspace::filter::preset::tests::browse_search_split_inverse`
- `workspace::filter::preset::tests::code_index_hard_skips_dependency_dirs_without_search_exclude`
- `workspace::filter::preset::tests::no_ignore_skips_binary_unlike_unfiltered`
- `workspace::filter::preset::tests::prune_product_internal_presets`
- `workspace::filter::preset::tests::watcher_and_unfiltered_never_honor_gitignore`
- `workspace::filter::walk::tests::agent_text_skips_nested_litecode_not_walk_root`
- `workspace::filter::walk::tests::agent_walk_reads_the_knowledge_root_and_skips_the_rest`
- `workspace::filter::walk::tests::explorer_and_index_split_gitignore_in_walk`
- `workspace::filter::walk::tests::file_include_skips_non_matching_files_keeps_dirs`
- `workspace::filter::walk::tests::index_walk_excludes_discovery_and_product_not_target`
- `workspace::filter::walk::tests::search_walk_honors_gitignore_target`
- `workspace::filter::walk::tests::text_search_walk_skips_nul_binary`
- `workspace::filter::walk::tests::trailing_slash_search_exclude_prunes_directory_walk`
- `workspace::filter::workspace_excludes::tests::builtins_match_static_tables`
- `workspace::filter::workspace_excludes::tests::empty_code_index_extensions_stay_empty`
- `workspace::filter::workspace_excludes::tests::ensure_seeds_missing_file`
- `workspace::filter::workspace_excludes::tests::explorer_switch_roundtrips_through_file`
- `workspace::filter::workspace_excludes::tests::is_workspace_excludes_rel_matches_canonical`
- `workspace::filter::workspace_excludes::tests::legacy_file_without_explorer_field_defaults_false`
- `workspace::filter::workspace_excludes::tests::normalize_drops_blanks_comments_dupes`
- `workspace::filter::workspace_excludes::tests::normalize_strips_trailing_slashes`
- `workspace::filter::workspace_excludes::tests::path_triggers_index_sync_for_excludes_and_gitignore`
- `workspace::filter::workspace_excludes::tests::persist_roundtrip_does_not_require_cache`
- `workspace::filter::workspace_excludes::tests::reload_from_disk_activates_valid_file`
- `workspace::filter::workspace_excludes::tests::reload_from_disk_reads_code_index_extensions`
- `workspace::filter::workspace_excludes::tests::reload_keeps_cache_on_invalid_json`
- `workspace::filter::workspace_excludes::tests::reload_keeps_cache_when_file_missing`
- `workspace::tool_path::tests::agent_and_lsp_share_safe_scope`
- `workspace::tool_path::tests::human_relative_rejects_absolute_paths`
- `workspace::tool_path::tests::relative_path_uses_workspace_root`
- `workspace::tool_path::tests::relative_traversal_is_rejected_in_all_modes`

### LSP · 冷启动/导航/路径（41）
- `lsp::conn::tests::cache_covers_sync_requires_matching_version`
- `lsp::conn::tests::configuration_ack_is_queued`
- `lsp::conn::tests::content_modified_passthrough_rpc_id_keeps_raw_error`
- `lsp::conn::tests::content_modified_plain_request_returns_empty_once`
- `lsp::conn::tests::content_modified_resolve_is_cancelled_once`
- `lsp::conn::tests::diagnostics_are_current_rejects_stale_cache`
- `lsp::conn::tests::dispatch_out_of_order_responses`
- `lsp::conn::tests::edit_fix_must_not_echo_previous_round_error`
- `lsp::conn::tests::is_content_modified_matches_only_32801`
- `lsp::conn::tests::publish_diagnostics_fans_out_to_subscribers`
- `lsp::conn::tests::success_is_value`
- `lsp::conn::tests::wait_returns_current_errors_without_blocking`
- `lsp::format::tests::display_path_tries_roots_in_order`
- `lsp::format::tests::format_locations_text_uses_relative_path`
- `lsp::hub::tests::split_want_rev_absent_is_none`
- `lsp::server::tests::content_length_parses_and_caps`
- `lsp::server::tests::diagnostics_gate_table`
- `lsp::server::tests::eof_position_tracks_utf16`
- `lsp::server::tests::index_not_ready_error_maps_to_unavailable_wording`
- `lsp::server::tests::nav_empty_symbols_by_array_len`
- `lsp::tests::command_parts_preserves_quoted_program_path`
- `lsp::tests::configuration_response_roots_vs_nested`
- `lsp::tests::coverage_requires_active_and_configured_command`
- `lsp::tests::deferred_activate_runs_no_probe_until_first_need`
- `lsp::tests::detect_rust_from_cargo_toml`
- `lsp::tests::error_diagnostics_block_silent_without_errors`
- `lsp::tests::extract_locations_accepts_location_and_location_link`
- `lsp::tests::file_to_uri_strips_verbatim_prefix`
- `lsp::tests::instance_statuses_empty_before_spawn`
- `lsp::tests::lifecycle_status_serializes_snake_case`
- `lsp::tests::normalize_aliases`
- `lsp::tests::normalize_windows_verbatim_file_uri`
- `lsp::tests::probe_failure_deactivates_hub_and_reports_original_error`
- `lsp::tests::probe_runs_once_per_activation`
- `lsp::tests::publish_diagnostics_uri_matches_mangled_verbatim`
- `lsp::tests::restart_after_failed_probe_says_hub_not_active`
- `lsp::tests::server_map_builtin_contains_rust_analyzer`
- `lsp::tests::short_feedback_reason_truncates_long_errors`
- `lsp::tests::spawn_ls_waits_for_sweep_gate`
- `lsp::tests::stop_clears_active_and_coverage`
- `lsp::tests::strip_verbatim_drive_and_unc`

### terminal · agent_jobs（19）
- `terminal::agent_jobs::tests::alloc_pipe_job_id_skips_leftover_logs_and_call_id_colons`
- `terminal::agent_jobs::tests::attached_exit_skips_mailbox_and_log_name_ignores_call_id`
- `terminal::agent_jobs::tests::empty_call_id_does_not_register_waiter`
- `terminal::agent_jobs::tests::jobs_changed_fires_on_insert_finish_and_wait`
- `terminal::agent_jobs::tests::kill_registered_kills_pid_and_marks_user_killed`
- `terminal::agent_jobs::tests::mark_user_kill_before_finish_keeps_mailbox_notice_user_killed`
- `terminal::agent_jobs::tests::tail_view_uses_full_window_then_frozen_tail`
- `terminal::agent_jobs::tests::undetached_finish_skips_mailbox_and_exit_handler`
- `terminal::agent_jobs::tests::wait_async_takes_detached_exit_from_mailbox`
- `terminal::agent_jobs::tests::wait_async_times_out_while_job_stays_alive`
- `terminal::agent_jobs::tests::wait_async_yields_so_sibling_can_run`
- `terminal::agent_jobs::tests::wire_snapshot_keeps_running_jobs_and_waiters`
- `terminal::ansi::tests::removes_osc_with_st_terminator`
- `terminal::shell::tests::default_shell_is_platform_native`
- `terminal::shell::tests::shell_label_uses_the_program_stem`
- `terminal::tee::tests::file_stops_at_max_and_flags_truncation`
- `terminal::tee::tests::trim_to_tail_aligns_to_next_newline`
- `terminal::tests::display_output_path_strips_workspace_root`
- `terminal::tests::ids_are_unguessable_uuid_and_distinct`

### code_search 索引（100）
- `engines::code_search::atomic::tests::append_truncates_uncommitted_tail`
- `engines::code_search::atomic::tests::bytes_roundtrip`
- `engines::code_search::atomic::tests::crash_between_renames_leaves_prefix_and_sweep_clears_tmp`
- `engines::code_search::atomic::tests::failure_while_staging_keeps_old_files`
- `engines::code_search::atomic::tests::set_replaces_in_order_and_leaves_no_tmp`
- `engines::code_search::build::tests::build_index_skips_non_scannable`
- `engines::code_search::build::tests::embedding_corpus_honors_index_gitignore_switch`
- `engines::code_search::build::tests::scannable_respects_policy`
- `engines::code_search::chunk::tests::chunks_overlap_by_twenty_lines`
- `engines::code_search::code_tokenize::tests::camel_case_keeps_original_and_parts`
- `engines::code_search::code_tokenize::tests::extract_identifiers_skips_plain_english`
- `engines::code_search::code_tokenize::tests::query_terms_expand_identifiers`
- `engines::code_search::code_tokenize::tests::snake_case_splits`
- `engines::code_search::cool_tests::done_monotonic_from_enqueue_to_end`
- `engines::code_search::cool_tests::drop_embedder_and_index_reload_for_search`
- `engines::code_search::cool_tests::incremental_refresh_total_exact_with_delete_and_reindex`
- `engines::code_search::cool_tests::maybe_cool_respects_zero_idle`
- `engines::code_search::embed::tests::hash_embedder_produces_unit_vectors`
- `engines::code_search::embed::tests::source_tree_fallback_is_debug_only`
- `engines::code_search::enclosing::tests::an_old_type_only_impl_chain_resolves_when_one_trait_impl_matches`
- `engines::code_search::enclosing::tests::ancestor_caps_long_function_and_reports_remaining`
- `engines::code_search::enclosing::tests::ancestor_near_end_falls_back_none`
- `engines::code_search::enclosing::tests::ancestor_unsupported_lang_is_none`
- `engines::code_search::enclosing::tests::c_function`
- `engines::code_search::enclosing::tests::cpp_namespace_class_method`
- `engines::code_search::enclosing::tests::csharp_namespace_class_method`
- `engines::code_search::enclosing::tests::duplicate_chain_is_ambiguous`
- `engines::code_search::enclosing::tests::go_method_on_type`
- `engines::code_search::enclosing::tests::java_class_method`
- `engines::code_search::enclosing::tests::out_of_range_line_falls_back_empty`
- `engines::code_search::enclosing::tests::python_class_method`
- `engines::code_search::enclosing::tests::rust_free_function`
- `engines::code_search::enclosing::tests::rust_lists_scopes_and_keeps_trait_impls_apart`
- `engines::code_search::enclosing::tests::rust_nested_impl_method_breadcrumb`
- `engines::code_search::enclosing::tests::typescript_and_python_scopes_resolve`
- `engines::code_search::enclosing::tests::typescript_class_method`
- `engines::code_search::enclosing::tests::unsupported_extension_falls_back_empty`
- `engines::code_search::facade::tests::human_search_text_column_uses_lexical_lane`
- `engines::code_search::facade::tests::include_semantic_defaults_true_in_json`
- `engines::code_search::facade::tests::include_semantic_false_parsed`
- `engines::code_search::fs_notify::tests::queue_scannable_path`
- `engines::code_search::index_status::tests::corpus_failed_reasons`
- `engines::code_search::index_status::tests::decide_index_work_first_desired_and_incompatible`
- `engines::code_search::index_status::tests::disk_status_absent_then_needs_rebuild_shell`
- `engines::code_search::index_status::tests::index_work_serializes_kind_tag`
- `engines::code_search::index_status::tests::job_file_overrides_to_building`
- `engines::code_search::index_status::tests::queue_embed_dirty_keeps_deletes_on_update`
- `engines::code_search::index_status::tests::queue_marks_stale_when_ready`
- `engines::code_search::index_status::tests::removed_legacy_helpers_stay_gone`
- `engines::code_search::index_status::tests::stale_running_job_is_reclaimed_and_requeued`
- `engines::code_search::lexical::tests::cancelled_agent_search_stops_before_walking`
- `engines::code_search::lexical::tests::context_lines_attached`
- `engines::code_search::lexical::tests::include_and_exclude_globs`
- `engines::code_search::lexical::tests::include_brace_pattern`
- `engines::code_search::lexical::tests::lexical_lane_source_never_calls_text_index_accelerator`
- `engines::code_search::lexical::tests::literal_escapes_regex_metacharacters`
- `engines::code_search::lexical::tests::literal_search_finds_line`
- `engines::code_search::lexical::tests::max_matches_is_global`
- `engines::code_search::lexical::tests::multiline_cross_line`
- `engines::code_search::lexical::tests::no_path_rg_dependency`
- `engines::code_search::lexical::tests::search_includes_unignored_hidden_files`
- `engines::code_search::lexical::tests::search_respects_excludes_keeps_hidden`
- `engines::code_search::lexical::tests::skips_binary_files_with_nul`
- `engines::code_search::lexical::tests::text_search_respects_gitignore`
- `engines::code_search::lexical::tests::unfiltered_preset_includes_editor_excluded_ignored_and_hidden_files`
- `engines::code_search::lexical::tests::whole_word_filter`
- `engines::code_search::meta::tests::embedder_id_mismatch_requires_rebuild`
- `engines::code_search::meta::tests::init_creates_meta_without_vectors`
- `engines::code_search::meta::tests::pipeline_version_change_requires_rebuild`
- `engines::code_search::reconcile::tests::allowlist_change_reconciles_without_rebuild`
- `engines::code_search::reconcile::tests::diff_is_stat_only_and_generic`
- `engines::code_search::reconcile::tests::files_without_chunks_are_accounted_not_requeued`
- `engines::code_search::reconcile::tests::index_cold_scan_queues_without_reload`
- `engines::code_search::reconcile::tests::index_cold_without_stamps_reconciles_on_load`
- `engines::code_search::reconcile::tests::reconcile_queues_new_modified_and_deleted`
- `engines::code_search::reconcile::tests::restart_uses_persisted_stamps_and_catches_offline_changes`
- `engines::code_search::reconcile::tests::serve_scan_from_stamps_queues_without_runtime`
- `engines::code_search::reconcile::tests::stamp_fast_path_skips_unchanged`
- `engines::code_search::reconcile::tests::touch_without_content_change_does_not_reembed`
- `engines::code_search::retrieve::tests::code_cc_alpha_is_workspace_only_constant`
- `engines::code_search::retrieve::tests::glob_filters_ann_leg`
- `engines::code_search::retrieve::tests::hybrid_returns_path_and_lines`
- `engines::code_search::semantic_engine::tests::search_returns_hits_for_indexed_content`
- `engines::code_search::stamps::tests::root_change_invalidates`
- `engines::code_search::stamps::tests::round_trips_stamped_and_unstamped_paths`
- `engines::code_search::store::tests::round_trip_save_load`
- `engines::code_search::store::tests::save_empty_index_creates_vectors_file`
- `engines::code_search::store::tests::save_load_unicode_workspace_path`
- `engines::code_search::wp143_tests::crash_between_compaction_renames_keeps_chunks_and_vectors_one_to_one`
- `engines::code_search::wp143_tests::kill_resume_first_build_embeds_only_remaining`
- `engines::code_search::wp143_tests::kill_resume_rebuild_embeds_only_remaining`
- `engines::code_search::wp143_tests::uncommitted_append_tail_is_cut_on_load`
- `engines::code_search_engine::tests::handshake_closed_stdout_is_worker_death`
- `engines::code_search_engine::tests::notify_buffers_while_cold`
- `engines::code_search_engine::tests::stop_during_warmup_kills_worker`
- `engines::code_search_engine::tests::worker_pid_available_during_warmup`
- `engines::code_search_ipc::protocol::tests::ping_result_defaults_empty_device`
- `engines::code_search_ipc::protocol::tests::ping_result_roundtrip_cuda`
- `engines::code_search_ipc::protocol::tests::refresh_params_empty_defaults_all`
- `engines::code_search_ipc::protocol::tests::refresh_params_roundtrip_code_and_session`

## 为凑 ≤1000 删掉的高风险测试（名称命中并发/锁/封口/数据不丢/安全/预算等关键词，共 120 条）

### session::data（10）
- `session::data::sqlite::session::tests::a_pending_tool_result_is_a_new_append_even_when_an_older_result_matches`
- `session::data::sqlite::session::tests::commit_of_unchanged_in_progress_payload_still_seals_the_row`
- `session::data::sqlite::session::tests::commit_orphan_cleanup_leaves_memory_untouched_when_tx_fails`
- `session::data::sqlite::session::tests::incremental_surface_matches_fold_after_compact_and_append`
- `session::data::sqlite::session::tests::load_events_roundtrip_append_origin_is_contiguous_seq`
- `session::data::sqlite::session::tests::pad_unanswered_calls_inserts_after_call_block`
- `session::data::sqlite::session::tests::revert_preview_is_text_not_json_brace`
- `session::data::sqlite::session::tests::seal_in_progress_items_marks_incomplete_and_final`
- `session::data::sqlite::session::tests::seal_stream_item_is_refused_on_a_settled_row`
- `session::data::sqlite::session::tests::snip_stale_results_removes_orphan_outputs`

### client_protocol::controller（9）
- `client_protocol::controller::compact_item_wire_tests::cancel_seal_restamps_changed_seqs_without_next_seq_growth`
- `client_protocol::controller::compact_item_wire_tests::commit_after_the_stream_settled_finds_nothing_to_seal_or_append`
- `client_protocol::controller::compact_item_wire_tests::compact_succeeded_clears_meter_and_refreshes_local_estimate`
- `client_protocol::controller::compact_item_wire_tests::compact_succeeded_emits_compacted_buffer_row`
- `client_protocol::controller::compact_item_wire_tests::reopen_uses_latest_bill_and_compact_retires_it`
- `client_protocol::controller::compact_item_wire_tests::revert_then_append_delivers_new_seq_using_persisted_high_water`
- `client_protocol::controller::compact_item_wire_tests::second_compact_appends_another_checkpoint_item`
- `client_protocol::controller::compact_item_wire_tests::snapshot_compact_eligible_follows_provider_occupancy`
- `client_protocol::controller::merged_channel_tests::merged_overflow_drops_frame_and_signals_lagged_not_blocking`

### engines::code_search（9）
- `engines::code_search::atomic::tests::append_truncates_uncommitted_tail`
- `engines::code_search::atomic::tests::crash_between_renames_leaves_prefix_and_sweep_clears_tmp`
- `engines::code_search::index_status::tests::queue_marks_stale_when_ready`
- `engines::code_search::index_status::tests::stale_running_job_is_reclaimed_and_requeued`
- `engines::code_search::lexical::tests::cancelled_agent_search_stops_before_walking`
- `engines::code_search::lexical::tests::include_brace_pattern`
- `engines::code_search::lexical::tests::literal_escapes_regex_metacharacters`
- `engines::code_search::wp143_tests::crash_between_compaction_renames_keeps_chunks_and_vectors_one_to_one`
- `engines::code_search::wp143_tests::uncommitted_append_tail_is_cut_on_load`

### client_protocol::connection（7）
- `client_protocol::connection::ask_loop_tests::cancel_aborts_pending_ask`
- `client_protocol::connection::compact_error_tests::compact_nothing_and_cancelled_unchanged`
- `client_protocol::connection::compact_error_tests::compact_real_failure_is_compaction_failed_not_retryable`
- `client_protocol::connection::pending_ask_tests::abort_all_pending_is_aborted_not_deny`
- `client_protocol::connection::pending_ask_tests::concurrent_asks_resolve_independently`
- `client_protocol::connection::pending_ask_tests::matching_reject_resolves_deny`
- `client_protocol::connection::pending_ask_tests::retain_pending_on_disconnect_does_not_deny`

### context_pipeline::compact（5）
- `context_pipeline::compact::tests::compact_request_disables_thinking_and_uses_caller_cap`
- `context_pipeline::compact::tests::compact_request_observes_cancel_while_waiting`
- `context_pipeline::compact::tests::compact_request_times_out_on_silent_provider`
- `context_pipeline::compact::tests::compact_summary_drops_reasoning_items`
- `context_pipeline::compact::tests::compact_summary_empty_when_only_reasoning`

### engines::session_search（5）
- `engines::session_search::dense_parity::an_appended_dense_index_equals_a_clean_rebuild`
- `engines::session_search::tests::agent_view_spills_the_remainder_into_a_named_file`
- `engines::session_search::tests::context_window_exclude_drops_live_seqs_keeps_archive`
- `engines::session_search::tests::lane_state::a_stale_index_is_caught_up_behind_the_query`
- `engines::session_search::tests::lane_state::concurrent_searches_agree_and_none_is_left_unanswered`

### llm::codec（5）
- `llm::codec::http::tests::send_cancellable_returns_canceled_when_token_fires`
- `llm::codec::messages::tests::compaction_keeps_thinking_signed_against_the_summary`
- `llm::codec::responses::tests::cancel_before_any_event_is_canceled`
- `llm::codec::responses::tests::compact_summary_does_not_send_an_empty_id`
- `llm::codec::responses::tests::plain_model_sends_the_authority_shape`

### session::snapshot（5）
- `session::snapshot::tests::max_file_revert_seq_is_highest_nonempty_ok_patch`
- `session::snapshot::tests::restore_missing_track_ref_is_unavailable`
- `session::snapshot::tests::sequential_snapshot_track_is_clean`
- `session::snapshot::tests::snapshot_restore_seq_stem_is_not_user_index`
- `session::snapshot::tests::workspace_snapshot_lock_is_shared_per_workspace`

### workspace::file_ops（5）
- `workspace::file_ops::tests::atomic_create_bytes_creates_new_file`
- `workspace::file_ops::tests::atomic_create_fails_if_exists_without_clobber`
- `workspace::file_ops::tests::atomic_write_overwrites_existing_content`
- `workspace::file_ops::tests::atomic_write_preserves_hardlink_inode`
- `workspace::file_ops::tests::atomic_write_sets_standard_permissions_on_create`

### serve::run（4）
- `serve::run::tests::case_a_loopback_require_auth_with_token_ok`
- `serve::run::tests::case_b_non_loopback_require_auth_with_token_ok`
- `serve::run::tests::loopback_only_rejects_non_loopback_even_with_auth`
- `serve::run::tests::loopback_without_auth_still_ok`

### session::surface（4）
- `session::surface::tests::failed_append_does_not_enter_the_log`
- `session::surface::tests::job_exit_appends_as_spine_item`
- `session::surface::tests::session_event_seq_is_required_not_optional`
- `session::surface::tests::transcript_is_append_origin_sorted_by_seq_not_slice_order`

### types::death_list_gate（4）
- `types::death_list_gate::session_seq_g1_envelope_vocab_matches_mental_model`
- `types::death_list_gate::session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept`
- `types::death_list_gate::session_seq_g4_wire_speaks_seq`
- `types::death_list_gate::session_seq_g6_derived_paths_use_surface_not_pointers`

### client_protocol::project（3）
- `client_protocol::project::wp186_pending_permissions_tests::pending_permissions_matches_frozen_shape_fields`
- `client_protocol::project::wp186_pending_permissions_tests::snapshot_always_serializes_pending_permissions_even_when_empty`
- `client_protocol::project::wp186_pending_permissions_tests::turn_awaiting_permission_derived_from_list_len`

### lsp::conn（3）
- `lsp::conn::tests::content_modified_resolve_is_cancelled_once`
- `lsp::conn::tests::diagnostics_are_current_rejects_stale_cache`
- `lsp::conn::tests::wait_returns_current_errors_without_blocking`

### tools::bash_safety（3）
- `tools::bash_safety::tests::bypass_backtick_rm_is_denied`
- `tools::bash_safety::tests::bypass_eval_rm_is_denied`
- `tools::bash_safety::tests::bypass_nested_obfuscation_is_denied`

### tools::code_search（3）
- `tools::code_search::tests::agent_view_tight_budget_drops_to_context_without_breadcrumb`
- `tools::code_search::tests::agent_view_tiny_budget_truncates_hit_count`
- `tools::code_search::tests::fence_escapes_backticks_in_source`

### client_protocol::protocol（2）
- `client_protocol::protocol::tests::buffer_load_result_is_a_seq_window`
- `client_protocol::protocol::tests::buffer_state_wire_is_seq_cursor_not_count_window`

### config::schema（2）
- `config::schema::mcp_server_definition_tests::omitted_timeout_defaults_to_sixty`
- `config::schema::mcp_server_definition_tests::zero_timeout_means_default`

### ptc::engine（2）
- `ptc::engine::settle_tests::cancel_during_drain_keeps_the_value`
- `ptc::engine::settle_tests::timeout_during_drain_keeps_the_returned_value`

### reminder::sources（2）
- `reminder::sources::tests::background_restore_gates_on_compaction_and_content`
- `reminder::sources::tests::tasks_restore_only_after_compaction_and_once`

### session::manager（2）
- `session::manager::child_session_tests::open_child_and_remove_parent_cascades_in_memory`
- `session::manager::child_session_tests::remove_parent_releases_running_child_turn_guard_and_cancels_child`

### tool::output（2）
- `tool::output::tests::finalize_spills_text_without_touching_media_parts`
- `tool::output::tests::spill_and_resolve_roundtrip`

### tools::webfetch（2）
- `tools::webfetch::tests::empty_url_is_hard_invalid_input`
- `tools::webfetch::tests::not_warmed_is_hard_unavailable`

### workspace::filter（2）
- `workspace::filter::path_glob::tests::brace_expansion_ts_tsx`
- `workspace::filter::path_glob::tests::comma_inside_braces_not_split`

### authority::tests（1）
- `authority::tests::authority_item_roundtrip_function_call`

### config::settings_writer（1）
- `config::settings_writer::tests::set_key_rejects_removed_auth_token`

### context_pipeline::budget（1）
- `context_pipeline::budget::tests::manual_compact_gate_is_inclusive_at_thirty_percent`

### lsp::server（1）
- `lsp::server::tests::index_not_ready_error_maps_to_unavailable_wording`

### lsp::tests（1）
- `lsp::tests::error_diagnostics_block_silent_without_errors`

### optional::exa_mcp（1）
- `optional::exa_mcp::tests::mcp_url_appends_settings_key`

### platform_knobs::tests（1）
- `platform_knobs::tests::max_defaults_to_the_standard_budget`

### provider_catalog::tests（1）
- `provider_catalog::tests::seed_context_and_output_follow_the_budget_policy`

### runtime::diagnostics（1）
- `runtime::diagnostics::tests::append_tool_audit_roundtrip`

### runtime::stream_projection（1）
- `runtime::stream_projection::tests::update_stream_item_failure_fails_settle_and_still_seals`

### tool::executor（1）
- `tool::executor::tests::cancellable_tools_opt_in_and_the_default_is_off`

### tools::code（1）
- `tools::code::render::tests::default_view_stays_inside_the_budget_without_legacy_sections`

### tools::custom（1）
- `tools::custom::tests::timeout_kills_grandchild_process`

### tools::edit（1）
- `tools::edit::tests::edit_declares_a_thirty_second_timeout`

### tools::grep（1）
- `tools::grep::tests::wide_search_never_writes_a_spill_file`

### tools::litecode_workspace（1）
- `tools::litecode_workspace::tests::read_commands_are_concurrency_safe`

### tools::write（1）
- `tools::write::expected_hash_tests::write_expected_hash_on_missing_file_does_not_create`

### types::error（1）
- `types::error::tests::compaction_failed_display_matches_empty_summary_path`

### workspace::service（1）
- `workspace::service::tests::refuses_symlink_copy_escaping_workspace`

### workspace::tool_path（1）
- `workspace::tool_path::tests::relative_traversal_is_rejected_in_all_modes`
