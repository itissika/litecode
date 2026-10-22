# WP-208 核心链路 → 守护测试（基准 3acfe31，六域人工核对汇总）

> **初稿（脚本按关键词/整文件生成，commit 28d3c85）已作废。** 本版由 Tools / Runtime / Session / 上下文 / Provider / IDE 六个域逐条读测试体人工核对（`review-*.md`），这里只做汇总与去重；未跑 cargo，未删任何测试。
> 旧 `coverage-map.md` 同样作废。
> 测试名均已对 3acfe31 源码逐条校验存在。去重规则：同一测试只在归属域的链路下列一次；其他域引用处标「→ 归 X」。

## 1. 核心链路（按模块）

### tools

**Tools·C1** read：1-based start_line/limit 分页，默认页有界并提示下一页；越 EOF（含空文件）resolve 空 lines+message「共 N 行」；缺文件 ToolError not_found；长行在 text 内截断；工作区外路径可读并标注；取消在分块间生效。（守护 8）
- `src/tools/read.rs::test_read_with_start_line_and_limit`
- `src/tools/read.rs::test_read_default_limit_pages`
- `src/tools/read.rs::test_read_start_line_past_end_resolves_empty`
- `src/tools/read.rs::test_read_empty_file_past_end_resolves_empty`
- `src/tools/read.rs::read_missing_file_is_not_found`
- `src/tools/read.rs::test_read_long_line_is_capped_in_text`
- `src/tools/read.rs::outside_workspace_paths_noted`
- `src/tools/read.rs::cancelled_read_stops_between_chunks`

**Tools·C2** grep：结果为有界 hits（limit 触顶进 message、不落 spill 文件）；非法 regex / 空 glob 为 invalid_input；排除树/二进制/空 glob 等结果态走 resolve+message。（守护 7）
- `src/tools/grep.rs::limit_bounds_hits_and_notes_truncation`
- `src/tools/grep.rs::wide_search_never_writes_a_spill_file`
- `src/tools/grep.rs::a_bare_brace_is_invalid_regex`
- `src/tools/grep.rs::blank_glob_is_invalid_input`
- `src/tools/grep.rs::grep_message_path_excluded`
- `src/tools/grep.rs::grep_message_binary_file`
- `src/tools/grep.rs::grep_message_glob_empty`

**Tools·C3** glob：无 / 的模式按文件名任意深度匹配；含 .. 的模式 invalid_input；默认尊重 gitignore；触顶截断进 message；取消在遍历前生效。（守护 5）
- `src/tools/glob.rs::basename_glob_recurses`
- `src/tools/glob.rs::parent_dir_pattern_is_invalid_input`
- `src/tools/glob.rs::default_filters_hide_gitignored`
- `src/tools/glob.rs::truncates_with_message`
- `src/tools/glob.rs::cancelled_glob_stops_before_walking`

**Tools·C4** write：expected_hash 不符 → stale 且不落盘、缺文件带 hash 不创建；create_only 遇已存在 → conflict 不覆盖；虚拟 session 路径只读；敏感路径在 run_code 内同样被 floor 拒绝。（守护 6）
- `src/tools/write.rs::write_expected_hash_mismatch_is_stale`
- `src/tools/write.rs::write_expected_hash_on_missing_file_does_not_create`
- `src/tools/write.rs::write_expected_hash_match_returns_file_ref`
- `src/tools/write.rs::test_write_create_only_refuses_overwrite`
- `src/tools/write.rs::virtual_session_path_is_read_only`
- `tests/code_surface.rs::nested_write_sensitive_path_is_denied_by_the_same_floor`

**Tools·C5** edit/apply_edits（WP-198 契约）：扁平 old/new，无 replace_all/expected_hash；唯一匹配替换、空 new 删除；未命中/多匹配 resolve ok:false+message（报行号、不提 replace_all）且不落盘；apply_edits 同 path 真串行、单条失败不中断且 results 等长；缩进仅差异按结构拒绝。（守护 12）
- `src/tools/edit/tests.rs::single_success_reports_edit_result`
- `src/tools/edit/tests.rs::empty_new_string_deletes_old`
- `src/tools/edit/tests.rs::unmatched_reports_no_match_without_applying`
- `src/tools/edit/tests.rs::edit_tool_wire_multiple_matches`
- `src/tools/edit/tests.rs::edit_failure_resolves_with_message`
- `src/tools/edit/tests.rs::edit_tool_schema_accepts_flat_shape_rejects_old`
- `src/tools/edit/tests.rs::edit_tool_cancel_before_write`
- `src/tools/edit/tests.rs::edit_tool_rejects_nul_bytes`
- `src/tools/apply_edits.rs::serial_skip_continues_results_same_len`
- `src/tools/apply_edits.rs::schema_rejects_retired_knobs`
- `src/tools/edit/python_indent.rs::two_space_vs_four_space_is_whitespace_only_reject`
- `src/tools/edit/matcher.rs::structural_prefix_and_indent_block_before_apply`

**Tools·C6** run/wait/kill：短命令前台返回 exit_code；超窗返回 exit_code=null 且进程续跑（promote）；资源锁只持有到 spawn（并行写不串行化，spawn 失败/future 丢弃/panic 都释放）；前台窗受 run_code deadline 封顶；wait 超时保持 null、退出后带码；kill 真杀进程；未知 id not_found。（守护 11）
- `src/tools/run.rs::fast_command_returns_exit_code`
- `src/tools/run.rs::long_command_returns_null_exit_and_keeps_job`
- `src/tools/run_lock_gate.rs::locks_release_after_spawn_while_command_still_runs`
- `src/tools/run_lock_gate.rs::four_parallel_write_runs_finish_in_about_one_second`
- `src/tools/run_lock_gate.rs::spawn_failure_releases_locks`
- `src/tools/run_lock_gate.rs::dropped_in_flight_future_releases_and_panic_unwind_releases`
- `src/tools/run_lock_gate.rs::window_is_capped_by_run_code_deadline`
- `src/tools/wait.rs::wait_until_exit_sets_code`
- `src/tools/wait.rs::wait_timeout_keeps_null_exit`
- `src/tools/kill.rs::kill_stops_process_and_notes_output`
- `src/tools/kill.rs::unknown_id_is_not_found`

**Tools·C7** LSP 七工具（WP-202）：七个 snake_case 名且仅 Code 面；定位入参 file_path+1-based line/UTF-16 character，越界/目录/根外 ToolError；workspace_symbol 仅 query 且每条带相对 path；空结果 resolve+[]+message；未暖 unavailable（非空成功）。（守护 11）
- `src/tools/lsp.rs::code_tool_names_are_snake_case_seven`
- `src/tools/lsp.rs::all_code_tools_are_code_surface_no_direct_lsp`
- `src/tools/lsp.rs::position_schema_requires_file_path_line_character`
- `src/tools/lsp.rs::workspace_symbol_schema_is_query_only`
- `src/tools/lsp.rs::character_out_of_range_is_hard_error`
- `src/tools/lsp.rs::character_utf16_multibyte`
- `src/tools/lsp.rs::absolute_outside_root_rejected`
- `src/tools/lsp.rs::directory_rejected`
- `src/tools/lsp.rs::finish_locations_empty_resolves_with_empty_array`
- `src/tools/lsp.rs::index_not_ready_rejects_unavailable_for_symbols_and_diagnostics`
- `src/tools/lsp.rs::finish_symbols_workspace_paths_relative`

**Tools·C8** webfetch/websearch（WP-203）：SSRF 拒 loopback/私网/100.64/IPv4-mapped，放行 198.18/15；非 http/私网/未 warm 为硬 ToolError（invalid_input/permission_denied/unavailable）；HTTP/Exa 失败软 resolve ok:false 带 url/status 或 query。（守护 9）
- `src/tools/webfetch.rs::ssrf_rejects_loopback_and_private_ips`
- `src/tools/webfetch.rs::ssrf_shared_cgnat_still_non_public`
- `src/tools/webfetch.rs::ssrf_allows_benchmarking_fake_ip_range`
- `src/tools/webfetch.rs::private_ip_is_hard_permission_denied`
- `src/tools/webfetch.rs::non_http_scheme_is_hard_invalid_input`
- `src/tools/webfetch.rs::not_warmed_is_hard_unavailable`
- `src/tools/webfetch.rs::soft_fail_shape_has_ok_false_without_error_level`
- `src/tools/websearch.rs::soft_fail_shape_resolves_ok_false`
- `src/tools/websearch.rs::not_warmed_surfaces_configuration_hint`

**Tools·C9** run_code 结果面（WP-200）：只渲染 head+logs+result，在预算内保尾并标注省略；成功/失败只计已结算调用，未结算单独旁注；by-tool data 不泄入 content；第二个 JS 参数在工具运行前拒绝；沙箱无 process/fs/module。（守护 11）
- `src/tools/code/render.rs::default_view_stays_inside_the_budget_without_legacy_sections`
- `src/tools/code/render.rs::large_result_keeps_the_tail_and_states_the_omission`
- `src/tools/code/render.rs::output_limit_mark_survives_an_overlong_last_line`
- `src/tools/code/render.rs::success_header_counts_failures_from_call_states`
- `src/tools/code/render.rs::failure_header_counts_and_marks_unsettled`
- `src/tools/code.rs::unfinished_call_stays_unsettled_even_when_the_host_later_records_ok`
- `src/tools/code.rs::permission_denial_is_counted_as_a_failed_settled_call`
- `src/tools/code.rs::render_run_output_attaches_by_tool_data_without_leaking_into_content`
- `src/tools/code.rs::a_second_javascript_argument_is_rejected_before_the_tool_runs`
- `tests/code_tool_runtime.rs::code_tool_captures_logs_and_enforces_output_ledger`
- `tests/code_tool_runtime.rs::code_tool_has_no_process_filesystem_or_module_surface`

**Tools·C17** 底层文件原子性（write/create_only 依赖）：并发 create/不覆盖写至多一个成功；跨设备覆盖移动成功。（守护 3）
- `tests/wp028_probe_s2_create_race.rs::probe_s2_create_file_race`
- `tests/wp028_probe_s2_create_race.rs::probe_s2_write_file_bytes_no_overwrite_race`
- `tests/wp028_probe_s1_xdev_move.rs::probe_s1_cross_device_overwrite_succeeds`

**Tools·C18** 检索类工具（code_search/session_search）：引擎非 Warm/失败时立即返回、不发起搜索、首行固定 sync 状态；Warm+Stale 仍可搜；session_search 超预算点名溢出文件、命中行号与虚拟 read 一致。（守护 7）
- `src/tools/code_search.rs::warming_returns_immediately_without_sleeping`
- `src/tools/code_search.rs::warm_failed_index_job_returns_error_not_hits`
- `src/tools/code_search.rs::not_ready_issues_no_search`
- `src/tools/code_search.rs::warm_stale_is_searchable_without_warm_wait`
- `src/tools/session_search.rs::semantic_unavailable_is_explicit_and_leads_with_sync_pct`
- `src/tools/session_search.rs::a_result_over_budget_names_the_file_that_holds_the_rest`
- `src/tools/session_search.rs::search_hit_line_matches_virtual_read`

### tool

**Tools·C10** Code 面投影与路由：Code 工具直调被拒（授权前）并经 run_code 成功；不可见工具 not_found 且不执行；嵌套调用保留工具自身权限、deny 不执行；无 data 契约/不可渲染工具不进两面；子深度按调用者可见集投影。（守护 9）
- `tests/code_surface.rs::direct_code_tools_are_refused_and_succeed_through_run_code`
- `tests/code_surface.rs::direct_edit_and_write_are_refused_before_authorization`
- `tests/code_surface.rs::code_call_of_invisible_tool_is_not_found_and_does_not_run`
- `tests/code_surface.rs::code_call_keeps_the_tools_own_permission_and_does_not_execute_a_deny`
- `tests/code_surface.rs::code_success_without_matching_data_is_an_internal_error`
- `tests/code_surface.rs::missing_or_unrenderable_code_tool_leaves_both_surfaces`
- `src/tool/registry.rs::code_gate_projects_every_depth_from_that_callers_visible_set`
- `src/tool/registry.rs::webfetch_websearch_project_to_code_not_direct`
- `src/tool/code_manifest.rs::rejects_tools_without_a_data_contract`

**Tools·C11** 嵌套调用取消/超时/grace 封口：取消封住在途嵌套调用与写；超 grace 记为 unsettled 且迟到的批准/拒绝不写盘；外层超时不覆盖已完成结果；等待审批不扣程序预算。（守护 8）
- `tests/code_surface.rs::cancellation_seals_in_flight_nested_calls`
- `tests/code_surface.rs::cancelled_run_code_seals_an_in_flight_write_and_edit`
- `tests/code_surface.rs::a_call_that_outlasts_the_grace_period_is_unsettled`
- `tests/code_surface.rs::grace_expiry_is_unsettled_and_the_later_denial_does_not_write`
- `tests/code_surface.rs::cancel_before_approval_does_not_write_later`
- `tests/code_surface.rs::outer_timeout_does_not_overwrite_a_completed_nested_result`
- `tests/code_surface.rs::approval_wait_does_not_consume_the_program_budget`
- `tests/code_surface.rs::nested_write_approval_denial_does_not_apply_the_edit`

**Tools·C12** 执行器与入参校验：已提交结果优先于超时/迟到取消；阻塞工具被管线超时打断；Code 调用未知参数 invalid_input，非法 JSON 不被吞；schema 校验必填/嵌套未知字段/oneOf。（守护 8）
- `src/tool/executor.rs::committed_result_wins_over_tool_timeout`
- `src/tool/executor.rs::completed_tool_result_wins_over_late_cancellation`
- `src/tool/executor.rs::blocking_tool_is_interrupted_by_pipeline_timeout`
- `src/tool/executor.rs::unknown_parameter_is_invalid_input`
- `src/tool/executor.rs::invalid_json_arguments_are_not_swallowed`
- `src/tool/schema_validate.rs::missing_required`
- `src/tool/schema_validate.rs::additional_properties_false_rejects_nested_unknown_field`
- `src/tool/schema_validate.rs::one_of_discriminated_union`

**Tools·C13** 资源锁：读共享/写独占，等待中的写不被后来读插队，同 key 写按提交序，多 key 排序防死锁，排队可取消，key 规范化；同路径不同拼写只有一个 create_only 赢家且读等写；审批挂起不预占也不阻塞他人；跨会话写锁无部分获取。（守护 15）
- `src/tool/resource_gate.rs::readers_share_and_a_writer_excludes_them`
- `src/tool/resource_gate.rs::a_waiting_writer_is_not_skipped_by_a_later_reader`
- `src/tool/resource_gate.rs::same_key_writes_finish_in_submission_order`
- `src/tool/resource_gate.rs::multi_key_acquires_in_sorted_order_do_not_deadlock`
- `src/tool/resource_gate.rs::cancel_while_queued_does_not_enter`
- `src/tool/resource_gate.rs::distinct_write_keys_do_not_block_each_other`
- `src/tool/resource_gate.rs::file_resource_key_folds_dot_parent_and_separators`
- `src/tool/resource_gate.rs::five_tools_declare_shared_reads_and_exclusive_writes`
- `src/tool/executor.rs::partition_splits_same_path_read_and_write`
- `src/tool/write_lock.rs::test_conflict_returns_holder`
- `src/tool/write_lock.rs::test_no_partial_acquire`
- `tests/code_surface.rs::conflicting_spellings_have_one_create_only_winner_and_a_read_waits_for_the_write`
- `tests/code_surface.rs::two_nested_writes_leave_one_complete_file_and_sequential_writes_both_apply`
- `tests/code_surface.rs::approval_pending_does_not_preclaim_the_same_path`
- `tests/code_surface.rs::approval_pending_does_not_lock_other_calls`

**Tools·C16** 契约快照：run_code SDK 文本+sha 与 fixture 逐字一致；各工具 data/error 形状锁定在 baseline。（守护 3）
- `src/tool/sdk_snapshot.rs::phase0_run_code_sdk_snapshot`
- `src/tool/sdk_snapshot.rs::sdk_hash_is_stable_for_the_same_tool_set_under_different_cwd_and_permission`
- `src/tools/baseline/mod.rs::phase0_tool_fixtures_lock_data_and_error`

### permission

**Tools·C14** 权限底线：floor（危险命令/敏感写）在 all 预设与 always 授权下仍硬拒且不问；safe 只读命令放行其余 deny；子代理 ask→deny；首条匹配规则胜出；always 授权按 rule+agent 限定；sink 取消为 aborted 非 deny；保留名/run_code 同名不得绕过（按 transport 身份）；命令混淆绕过被拒。（守护 28）
- `src/permission/floor.rs::run_hits_the_dangerous_command_floor`
- `src/permission/evaluate.rs::first_matching_rule_wins`
- `src/permission/engine.rs::subagent_turns_safe_ask_into_deny`
- `src/permission/engine.rs::reserved_names_keep_the_safe_binding`
- `src/permission/engine.rs::a_tool_named_run_code_keeps_safe_deny`
- `src/permission/sinks.rs::cancelling_sink_returns_aborted_not_deny`
- `src/permission/grants.rs::expired_grant_is_skipped`
- `tests/permission_pipeline.rs::safe_run_matrix_readonly_allow_others_deny_floor_hard_deny`
- `tests/permission_pipeline.rs::floor_sensitive_write_blocks_even_under_all_and_always_grant`
- `tests/permission_pipeline.rs::floor_blocks_sensitive_commands_under_unrestricted_all_preset`
- `tests/permission_pipeline.rs::safe_glob_outside_workspace_denies_via_path_arg`
- `tests/permission_pipeline.rs::grant_is_rule_and_agent_scoped`
- `tests/permission_pipeline.rs::authorize_ask_denied_by_sink`
- `tests/permission_pipeline.rs::authorize_ask_always_grants_same_rule_only`
- `tests/permission_pipeline.rs::authorize_floor_deny_ignores_always_grant`
- `tests/permission_pipeline.rs::authorize_subagent_ask_denied_without_sink`
- `tests/permission_pipeline.rs::authorize_ask_aborted_is_not_deny`
- `tests/permission_pipeline.rs::authorize_invalid_json_arguments_denied`
- `tests/code_surface.rs::code_transport_is_allowed_by_identity_not_by_name`
- `src/tool/registry.rs::custom_tools_cannot_take_command_or_retired_shell_names`
- `src/tool/registry.rs::mcp_inner_names_cannot_use_reserved_command_names`
- `src/tools/bash_safety.rs::bypass_backslash_escaped_rm_is_denied`
- `src/tools/bash_safety.rs::bypass_quote_wrapped_rm_is_denied`
- `src/tools/bash_safety.rs::bypass_command_substitution_rm_is_denied`
- `src/tools/bash_safety.rs::bypass_backtick_rm_is_denied`
- `src/tools/bash_safety.rs::bypass_eval_rm_is_denied`
- `src/tools/bash_safety.rs::bypass_nested_obfuscation_is_denied`
- `src/tools/bash_safety.rs::bypass_readonly_classification_is_not_foiled`

### mcp

**Tools·C15** MCP：每连接仅一次 initialize；JSON-RPC 读跳过通知/他 id，超时/取消标 dirty 并拒复用、重建后可用；崩溃后池重建、同 key 复用；kill/drop 回收孙进程；hub 挂起时 execute 保持 pending；启动失败不广告假工具。（守护 12）
- `src/mcp/client.rs::r1_skips_notification_before_response`
- `src/mcp/client.rs::r2_hang_times_out_and_marks_dirty`
- `src/mcp/client.rs::r3_cancel_marks_dirty_next_client_ok`
- `src/mcp/client.rs::kill_reaps_grandchild_process`
- `src/mcp/client.rs::drop_reaps_grandchild_process`
- `tests/mcp_integration.rs::initialize_is_single_shot_per_connection`
- `tests/mcp_integration.rs::call_tool_round_trips`
- `tests/mcp_integration.rs::pool_reuses_live_client_for_same_key`
- `tests/mcp_integration.rs::pool_rebuilds_after_crash`
- `tests/mcp_integration.rs::catalog_and_bind_exposes_echo_and_round_trips`
- `tests/mcp_integration.rs::execute_stays_pending_while_hang_blocks_on_hub`
- `src/tool/registry.rs::failed_mcp_start_does_not_advertise_dummy_tool`

### ptc

**Runtime·C1** PTC 值边界：JS→JSON 时 undefined 在参数中静默丢弃、在结果中剥离/置 null 并报 path（≤10 条 +N more），bigint/symbol/非安全整数带 path 硬错且不触达宿主（守护 6）
- `src/ptc/tests.rs::undefined_in_result_is_stripped_or_nulled_and_reported`
- `src/ptc/tests.rs::undefined_in_arguments_is_dropped_silently`
- `src/ptc/tests.rs::dropped_undefined_list_is_capped_with_more_count`
- `src/ptc/tests.rs::illegal_output_values`
- `src/ptc/tests.rs::illegal_arguments_skip_host`
- `src/ptc/tests.rs::host_integer_above_safe_range_is_rejected`

**Runtime·C2** PTC 预算/取消：死循环被 interrupt 打断；等待宿主/审批时暂停计费；取消时 in-flight 调用被取消，grace 后未结束调用记为未完成；成功返回也取消残留调用，未 await 调用被等待并记账（守护 7）
- `src/ptc/tests.rs::budget_interrupts_infinite_loop`
- `src/ptc/tests.rs::budget_pause_is_not_charged`
- `src/ptc/tests.rs::cancel_while_js_running_cancels_inflight`
- `src/ptc/tests.rs::cancel_while_awaiting_host`
- `src/ptc/tests.rs::cancel_grace_records_unfinished`
- `src/ptc/tests.rs::success_cancels_inflight_calls`
- `src/ptc/tests.rs::unawaited_calls_are_waited_and_recorded`

**Runtime·C3** PTC 成功不被改写：程序已 return 后 drain 遇超时/取消保留返回值，其他 drain 失败才替换成功（守护 5）
- `src/ptc/tests.rs::success_survives_budget_expiry_at_return`
- `src/ptc/engine.rs::success_is_not_rewritten_when_the_clock_is_already_exhausted`
- `src/ptc/engine.rs::timeout_during_drain_keeps_the_returned_value`
- `src/ptc/engine.rs::cancel_during_drain_keeps_the_value`
- `src/ptc/engine.rs::other_drain_failures_replace_success`

**Runtime·C4** PTC 栈/内存封口：栈溢出与 OOM 被 JS catch 后仍封为 ResourceLimit（Stack/Memory），不靠消息文本；超大稀疏数组分配前拒绝；深嵌套不崩（守护 8）
- `src/ptc/tests.rs::stack_limit`
- `src/ptc/tests.rs::stack_overflow_stays_sealed_when_the_program_catches_it`
- `src/ptc/tests.rs::stack_overflow_caught_then_immediate_return_is_still_sealed`
- `src/ptc/tests.rs::memory_limit`
- `src/ptc/tests.rs::memory_exhaustion_stays_sealed_when_the_program_catches_it`
- `src/ptc/tests.rs::caught_allocation_failure_is_memory_when_the_stack_watch_is_disarmed`
- `src/ptc/tests.rs::sparse_huge_array_rejected_before_allocation`
- `src/ptc/tests.rs::deep_nesting_does_not_crash`

**Runtime·C5** PTC 沙箱：无 fetch/require/process/setTimeout/std/os/Atomics/SAB，import 失败（守护 1）
- `src/ptc/tests.rs::no_ambient_capabilities`

**Runtime·C6** 工具错误语义：ToolError 可 catch 且 instanceof；未 catch 保留 error_code 并点名调用；用户异常不按消息分类；第二参数为可捕获 TypeError；caught 错误计数不列出（守护 6）
- `src/ptc/tests.rs::tool_error_is_catchable_and_instanceof`
- `src/ptc/tests.rs::uncaught_tool_error_keeps_code`
- `src/ptc/tests.rs::user_exceptions_are_not_classified_by_message`
- `tests/run_code_model_view.rs::caught_tool_error_is_counted_and_not_listed`
- `tests/run_code_model_view.rs::uncaught_tool_error_names_the_call`
- `tests/run_code_model_view.rs::a_second_argument_is_a_catchable_type_error`

**Runtime·C7** 输出预算：log 保前缀、最终值计入预算、截断只保可容纳前缀；进模型请求的结果受预算约束，超时保留 log 尾并点名预算（守护 6）
- `src/ptc/tests.rs::output_limit_keeps_log_prefix`
- `src/ptc/tests.rs::output_limit_counts_final_value`
- `src/ptc/output.rs::push_keeps_the_fitting_prefix`
- `src/ptc/tests.rs::dense_array_within_output_budget`
- `tests/run_code_model_view.rs::large_result_is_budgeted_in_the_request`
- `tests/run_code_model_view.rs::timeout_keeps_the_log_tail_and_names_the_budget`

### agent

**Runtime·C8** Agent loop：纯文本即停；function_call 先于 output 落盘；max_steps 封顶；取消/流中断时已有输出落盘、未执行工具补 interrupted pad；incomplete 调用不执行；持久化失败回滚；auto compact 失败/取消不调模型（守护 9，另引用 1）
- `tests/agent_loop.rs::single_text_response_stops_loop`
- `tests/agent_loop.rs::tool_step_persists_function_call_before_output`
- `tests/agent_loop.rs::max_steps_with_perpetual_tools`
- `tests/agent_loop.rs::cancelled_after_model_persists_output`
- `tests/agent_loop.rs::cancelled_after_stream_skips_tool_execution`
- `tests/agent_loop.rs::incomplete_function_call_is_not_executed`
- `tests/agent_loop.rs::persist_failure_rolls_back_working_set`
- `tests/agent_loop.rs::auto_compact_fail_stops_turn_without_model_call`
- `tests/agent_loop.rs::cancel_during_auto_compact_is_cancelled_without_model_call`
- `tests/agent_e2e_responses.rs::agent_e2e_incomplete_seals_text_to_disk` → 归 Provider·P16

### runtime

**Runtime·C9** 流式投影：流式 item 落到它打开的行；未确认 item 封 incomplete；provider 复用 id 开新行；身份冲突/终止重复 id fail-closed；写失败使调用失败但仍封口（守护 7）
- `src/runtime/stream_projection.rs::a_streamed_item_settles_into_the_row_it_opened`
- `src/runtime/stream_projection.rs::an_item_the_call_never_confirmed_settles_incomplete`
- `src/runtime/stream_projection.rs::a_call_that_reuses_a_provider_id_opens_a_new_row`
- `src/runtime/stream_projection.rs::same_call_identity_conflicts_fail_closed`
- `src/runtime/stream_projection.rs::terminal_duplicate_id_fails_closed`
- `src/runtime/stream_projection.rs::a_write_the_session_refuses_fails_the_call`
- `src/runtime/stream_projection.rs::update_stream_item_failure_fails_settle_and_still_seals`

**Runtime·C10** 请求前缀稳定（缓存）：已发出的请求前缀在 turn 内及跨 turn 重载不被改写；reminder 只追加；code 模式工具表稳定；compact 后恢复行位置正确（守护 8）
- `tests/turn_prefix_stability.rs::tool_loop_llm_request_prefix_json_is_stable`
- `tests/turn_prefix_stability.rs::code_mode_tool_list_prefix_stays_stable_across_the_turn`
- `tests/turn_prefix_stability.rs::job_exit_mid_turn_does_not_rewrite_already_sent_prefix`
- `tests/turn_prefix_stability.rs::persist_item_reused_message_id_must_not_rewrite_final_row`
- `tests/turn_prefix_stability.rs::persist_item_reused_function_call_id_must_not_rewrite_final_row`
- `tests/turn_prefix_stability.rs::next_turn_reload_must_keep_sent_row_when_provider_reuses_id`
- `tests/turn_prefix_stability.rs::state_reminders_only_append_across_steps`
- `tests/turn_prefix_stability.rs::restore_row_lands_after_compacted_and_prefix_stays`

**Runtime·C11** 排队消息/steer/stop_and_send：pending 在 tool loop seam 注入，最终回复期间到达则延长 turn；queue 模式不在 seam 消费、turn 结束后 FIFO 每条一 turn；steer 注入当前 turn；stop_and_send 取消后起新 turn（含启动窗口）；空闲 flush 只落一条 user 行（守护 10）
- `tests/pending_messages.rs::queued_message_is_injected_at_the_tool_loop_seam`
- `tests/pending_messages.rs::queued_message_during_final_response_extends_the_turn`
- `src/agent/core.rs::stop_check_continues_once_for_a_queued_message`
- `tests/wp098_follow_up_queue.rs::runtime_queue_not_drained_at_step_seam`
- `tests/wp098_follow_up_queue.rs::runtime_steer_still_injects_at_step_seam`
- `tests/wp098_follow_up_queue.rs::runtime_stop_and_send_cancels_then_runs_new_turn_no_notice`
- `src/runtime/pending_auto_turn.rs::idle_flush_reserves_claims_and_persists_one_user_row`
- `src/runtime/pending_auto_turn.rs::busy_session_leaves_the_queue_alone`
- `src/runtime/pending_auto_turn.rs::runtime_queue_multiple_runs_one_turn_each_fifo`
- `src/runtime/pending_auto_turn.rs::runtime_stop_and_send_starting_window`

**Runtime·C12** 自动 turn 唤醒：无订阅者不预留、忙时留 mailbox、UI kill 后空闲再提醒；子会话自动 turn 一律 deny sink（不得继承上次审批 sink）；子退出触发父 reminder（守护 6）
- `src/runtime/bash_auto_turn.rs::idle_without_subscribers_does_not_reserve`
- `src/runtime/bash_auto_turn.rs::busy_session_leaves_mailbox`
- `src/runtime/bash_auto_turn.rs::ui_kill_while_busy_prepares_user_stopped_reminder_once_idle`
- `src/runtime/subagent_auto_turn.rs::child_uses_deny_sink_regardless_of_last_sink`
- `src/runtime/pending_auto_turn.rs::child_uses_deny_sink_regardless_of_last_sink`
- `tests/subagent_session_isolation.rs::child_exit_triggers_parent_auto_turn_reminder`

### subagent

**Runtime·C13** 子会话生命周期/隔离：launch 建持久子会话并挂父链接；父事件通道不收子事件；绑定失败/缺 call_id 不留孤儿；删父级联；子 turn 读自身会话配置、身份不可被 set_primary 切换；结果从持久 turn 读取（守护 8）
- `tests/subagent_session_isolation.rs::subagent_launch_creates_durable_child_with_parent_link`
- `tests/subagent_session_isolation.rs::parent_event_channel_does_not_receive_child_turn_events`
- `tests/subagent_session_isolation.rs::failed_binding_aborts_orphan_child_session`
- `tests/subagent_session_isolation.rs::missing_call_id_scope_errors_without_creating_child`
- `tests/subagent_session_isolation.rs::remove_parent_cascades_child`
- `tests/subagent_child_turn_contract.rs::child_turn_reads_config_from_its_own_session_row`
- `tests/subagent_child_turn_contract.rs::child_identity_is_not_switchable_by_set_primary`
- `tests/subagent_child_turn_contract.rs::completed_child_result_is_read_from_the_durable_turn`

**Runtime·C14** 子代理并发：并发 send 只有一个赢；忙时排队不起 turn；拒绝未知/他人 id；send 后 stop 取消新 turn；wait 报 user stop；父取消不停后台子、stop 工具可停；worker panic 也释放会话；两 launch 并发（守护 8）
- `tests/subagent_send.rs::concurrent_sends_only_one_wins`
- `tests/subagent_send.rs::send_to_busy_child_queues_without_starting_a_turn`
- `tests/subagent_send.rs::send_rejects_unknown_and_foreign_ids`
- `tests/subagent_send.rs::send_then_stop_cancels_the_new_turn`
- `tests/subagent_send.rs::wait_returns_cancelled_when_user_stopped`
- `tests/subagent_tool_pipeline.rs::parent_cancel_does_not_stop_background_child_and_stop_tool_can`
- `tests/subagent_tool_pipeline.rs::worker_panic_finishes_job_and_releases_session`
- `tests/subagent_tool_pipeline.rs::pipeline_runs_two_launches_concurrently`

### terminal

**Runtime·C15** agent bash 作业：前台超时 promote 不杀、promote 后退出进 mailbox；前台取消杀进程且不进 mailbox；嵌套取消后 detach 续读；未 detach 结束不发 mailbox/handler；UI kill 杀 pid 并标 user_killed；wait 超时作业仍活；tee 超上限截断标记；订阅时水合作业、kill 后清空（守护 8）
- `src/terminal/agent_jobs.rs::foreground_timeout_promotes_without_killing`
- `src/terminal/agent_jobs.rs::foreground_cancel_kills_and_skips_mailbox`
- `src/terminal/agent_jobs.rs::detached_read_continues_after_nested_cancel`
- `src/terminal/agent_jobs.rs::undetached_finish_skips_mailbox_and_exit_handler`
- `src/terminal/agent_jobs.rs::kill_registered_kills_pid_and_marks_user_killed`
- `src/terminal/agent_jobs.rs::wait_async_times_out_while_job_stays_alive`
- `src/terminal/tee.rs::file_stops_at_max_and_flags_truncation`
- `tests/subscribe_bash_hydration.rs::subscribe_hydrates_bash_jobs_and_clears_after_kill`

**Runtime·C16** 终端安全/隔离：agent kill 拒绝人类 PTY 与猜测 id；id 为不可猜 uuid；WS 终端按 owner 隔离、断开清理；PTY 交互回显；跨块 CJK 不撕裂（守护 6）
- `src/terminal/mod.rs::kill_rejects_human_pty_id`
- `src/terminal/mod.rs::kill_unknown_or_guessed_id_is_rejected`
- `src/terminal/mod.rs::ids_are_unguessable_uuid_and_distinct`
- `tests/terminal_ws_isolation.rs::websocket_terminals_are_owner_isolated_and_disconnect_cleaned`
- `tests/terminal_pty_contract.rs::interactive_echo_round_trip`
- `src/terminal/pty.rs::drain_utf8_keeps_a_cjk_character_split_across_chunks`

### 架构守卫

**Runtime·C17** 架构/启动死名单（源码静态约束）：分层不越界；PTY 符号仅在 terminal 模块、前端无 node-pty；所有子进程 spawn 等 sweep gate、gate 无超时、sweep 先于 ready 且不内联（守护 12）
- `tests/architecture_layers.rs::session_layer_does_not_absorb_the_hub`
- `tests/architecture_layers.rs::tool_layer_does_not_finish_or_join_turns`
- `tests/architecture_layers.rs::tool_turn_helper_uses_human_reserve_then_spawn`
- `tests/architecture_layers.rs::runtime_and_protocol_do_not_open_child_sessions`
- `tests/architecture_layers.rs::tool_core_does_not_own_subagent_completion_delivery`
- `tests/terminal_bypass_death_list.rs::death_list_src_pty_symbols_only_in_terminal_module`
- `tests/terminal_bypass_death_list.rs::death_list_no_node_pty_in_desktop_or_web_package_json`
- `tests/terminal_bypass_death_list.rs::death_list_no_node_pty_require_in_desktop_web_src`
- `tests/startup_order_death_list.rs::every_spawn_waits_for_the_sweep_gate`
- `tests/startup_order_death_list.rs::sweep_gate_has_no_timeout`
- `tests/startup_order_death_list.rs::serve_run_arms_sweep_before_ready_path`
- `tests/startup_order_death_list.rs::serve_run_does_not_sweep_inline`

### session/data

**Session·C1** Session 数据层事务：创建/追加/删除幂等、乐观并发、故障回滚：每次变更走单事务，operation_id 幂等、stale revision 冲突、commit 前故障整体回滚、commit 后故障可幂等重放。（守护 9）
- `tests/session_data_crud.rs::create_append_meta_delete`
- `tests/session_data_crud.rs::idempotent_operation_id_returns_same_receipt`
- `tests/session_data_crud.rs::conflict_on_stale_revision`
- `tests/session_data_crud.rs::parent_delete_cascades_child`
- `tests/session_data_fault.rs::fault_before_commit_rolls_back`
- `tests/session_data_fault.rs::fault_after_commit_keeps_rows_and_operation_is_idempotent`
- `tests/session_data_concurrency.rs::same_revision_double_append_one_conflict`
- `tests/session_data_concurrency.rs::append_then_delete_late_commit_conflicts`
- `src/session/data/sqlite/ops.rs::persist_receipt_deleted_rejects_surviving_session_row`

**Session·C2** seq 单调与高水位（不复用已删 seq）：next_seq = max+1 持久化，truncate/revert/compact 后追加永不复用已删 seq，seal 不增 seq。（守护 7）
- `src/session/data/sqlite/session.rs::truncate_then_append_does_not_reuse_deleted_seqs`
- `src/session/data/sqlite/session.rs::reopen_after_truncate_keeps_high_water_and_appends_into_the_hole`
- `src/session/data/sqlite/session.rs::in_progress_item_is_retrievable_and_seal_keeps_seq`
- `src/session/data/sqlite/session.rs::load_events_roundtrip_append_origin_is_contiguous_seq`
- `src/session/data/sqlite/schema.rs::existing_session_db_backfills_next_seq_to_max_plus_one`
- `src/session/data/final_immutability_tests.rs::reopen_after_revert_does_not_reuse_a_deleted_seq`
- `tests/wire_seq_identity.rs::revert_then_append_does_not_reuse_deleted_seq`

**Session·C3** final 不可改 + 崩溃封口（冷恢复）：final 行永不重写；重启时 in-flight 行封为 incomplete、未答 call 补 interrupt 结果，后续追加接在其后。（守护 5）
- `src/session/data/final_immutability_tests.rs::a_finalised_row_is_never_rewritten`
- `src/session/data/final_immutability_tests.rs::a_restart_seals_rows_the_previous_run_left_in_flight`
- `src/session/data/final_immutability_tests.rs::a_restart_seals_an_in_flight_call_and_appends_its_interrupt_result`
- `src/session/data/final_immutability_tests.rs::sealing_a_settled_row_is_refused`
- `src/session/data/sqlite/session.rs::a_crashed_turn_is_sealed_incomplete_and_its_continuation_appends`

**Session·C4** compact 检查点与 fold 一致性：compact 只追加 checkpoint、不删历史 detail；reload 后 fold 结果与增量工作集一致；第二次 compact 追加新 checkpoint。（守护 7）
- `src/session/data/sqlite/session.rs::compact_never_deletes_historical_detail_rows`
- `src/session/data/sqlite/session.rs::writer_working_set_matches_fold_after_append_and_keep_recent_compact`
- `src/session/data/sqlite/session.rs::incremental_surface_matches_fold_after_compact_and_append`
- `src/session/data/sqlite/session.rs::compact_summary_agent_view_is_assistant_and_survives_reload`
- `src/session/data/sqlite/session.rs::revert_retaining_earlier_compact_restores_pointers`
- `tests/wire_seq_identity.rs::compact_then_revert_before_checkpoint_drops_compact_and_resets_pointers`
- `tests/wire_seq_identity.rs::second_compact_appends_new_seq_and_keeps_both_checkpoints`

### client_protocol

**Session·C5** 线协议 seq 身份 / snapshot 游标：wire 只发新 seq 行，snapshot.next_seq == DB max+1，runtime-only 流事件不长 seq、不发 snapshot。（守护 7）
- `tests/wire_seq_identity.rs::step_committed_emits_only_new_seq_and_snapshot_matches_max`
- `tests/wire_seq_identity.rs::turn_finished_snapshot_next_seq_matches_db_max_plus_one`
- `tests/wire_seq_identity.rs::runtime_only_stream_events_do_not_emit_snapshot_or_grow_seq`
- `tests/wire_seq_identity.rs::empty_session_cursor_is_minus_one_zero`
- `src/client_protocol/controller/mod.rs::cancel_seal_restamps_changed_seqs_without_next_seq_growth`
- `src/client_protocol/controller/mod.rs::revert_then_append_delivers_new_seq_using_persisted_high_water`
- `src/client_protocol/protocol.rs::buffer_load_result_is_a_seq_window`

**Session·C8** 权限往返与 pending_permissions（WP-186）：grant 按 request_id 精确匹配 allow/deny；断线/cancel 是 abort 而非 Deny；snapshot 恒序列化 pending_permissions，waiting_for_user 由列表长度派生。（守护 11）
- `src/client_protocol/connection.rs::matching_accept_resolves_allow`
- `src/client_protocol/connection.rs::matching_reject_resolves_deny`
- `src/client_protocol/connection.rs::retain_pending_on_disconnect_does_not_deny`
- `src/client_protocol/connection.rs::abort_all_pending_is_aborted_not_deny`
- `src/client_protocol/connection.rs::unknown_grant_is_buffered_as_stray_then_consumed`
- `src/client_protocol/connection.rs::concurrent_asks_resolve_independently`
- `src/client_protocol/project.rs::snapshot_always_serializes_pending_permissions_even_when_empty`
- `src/client_protocol/project.rs::turn_awaiting_permission_derived_from_list_len`
- `src/client_protocol/project.rs::pending_permissions_matches_frozen_shape_fields`
- `tests/client_protocol_permission.rs::ws_permission_bridge_abort_on_cancel_is_not_deny`
- `tests/client_protocol_permission.rs::ws_bridge_approval_round_trip_with_free_text`

**Session·C10** 会话列表/冷启动可见性：空新会话隐藏，有用户消息的会话在冷启动/坏行/缺 blob 下仍列出。（守护 5）
- `tests/session_list_empty_filter.rs::empty_new_session_stays_hidden`
- `tests/session_list_empty_filter.rs::cold_start_user_message_stays_listed`
- `tests/session_list_empty_filter.rs::malformed_transcript_rows_still_listed_after_reopen`
- `tests/session_list_empty_filter.rs::busy_empty_session_is_listed_until_cold_start`
- `src/client_protocol/ws_wp166_tests.rs::session_list_not_blocked_by_inflight_subscribe`

### session/manager

**Session·C6** Turn 生命周期（单 turn 互斥、取消、预约、revert/compact lease）：同 session 同时仅一 turn；取消在 starting 期也生效；compact/revert 与 turn 互斥；失败回滚 running 与 guard。（守护 9）
- `src/session/manager.rs::begin_turn_rejects_second_while_running`
- `src/session/manager.rs::cancel_during_starting_turn_cancels_on_start`
- `src/session/manager.rs::turn_reservation_closes_pre_spawn_idle_window`
- `src/session/manager.rs::compact_lease_atomically_blocks_turn_and_revert`
- `src/session/manager.rs::revert_during_running_turn_waits_for_finish_and_holds_exclusive_lease`
- `src/session/manager.rs::start_turn_fanout_spawn_failure_rolls_back_running_and_turn_guard`
- `src/session/manager.rs::finish_turn_clears_running_immediately_and_allows_rebegin`
- `tests/f2_lock_scope.rs::concurrent_commit_partial_does_not_lose_updates`
- `tests/f2_lock_scope.rs::session_mutation_does_not_hold_records_lock`

**Session·C7** 待发队列（pending messages）：enqueue/claim 不写 log；仅 running turn 可 claim；revert 丢弃失效队列、失败 revert 保留。（守护 5）
- `src/session/manager.rs::enqueue_remove_and_claim_never_touch_the_log`
- `src/session/manager.rs::claim_only_for_the_running_turn_and_restore_prepends`
- `src/session/manager.rs::a_reverted_log_drops_the_queue_it_made_meaningless`
- `src/session/manager.rs::a_failed_revert_keeps_the_queue`
- `tests/session_data_lifecycle.rs::queue_capacity_keeps_overflow_pending_until_cancelled`

**Session·C9** 子会话只读与删除级联（wp091/wp099）：用户对 child 的 run/retry/enqueue 被拒；删除 running 子/有 running 子的父被拒；删除留 tombstone 并级联。（守护 7）
- `src/client_protocol/connection.rs::wp091_agent_run_on_child_rejected_readonly`
- `src/client_protocol/connection.rs::wp091_delete_parent_with_running_child_returns_descendant_running`
- `src/client_protocol/connection.rs::wp091_delete_child_tombstone_and_notice`
- `src/session/manager.rs::delete_operation_is_blocked_by_active_descendant_turns`
- `src/session/delete.rs::tombstone_survives_reload`
- `src/session/delete.rs::remove_session_returns_descendants_sorted_root_last`
- `tests/session_delete.rs::delete_listed_idle_session_after_cold_start_without_subscribe`

**Session·C11** set-model（模型切换持久化）：模型切换与 control-plane 变更行同事务提交，失败一起回滚；仅接受 catalog 可选的引用。（守护 4）
- `src/session/manager.rs::model_change_commits_state_and_control_plane_row_together`
- `src/session/manager.rs::failed_model_change_rolls_back_state_and_event_together`
- `tests/settings_api.rs::set_session_model_persists_selectable_catalog_ref`
- `src/config/settings_writer.rs::model_switch_rejects_a_reference_the_catalog_does_not_declare`

### serve

**Session·C12** serve 绑定与 HTTP/WS 鉴权：非 loopback 必须 require-auth+token；API 只认 Bearer；WS 拒跨源、放行 localhost（含 ::1）。（守护 6）
- `tests/serve_bind_auth.rs::bind_auth_matrix`
- `src/serve/run.rs::loopback_only_rejects_non_loopback_even_with_auth`
- `tests/g3_auth.rs::api_rejects_query_token_and_accepts_bearer_only`
- `tests/g3_auth.rs::ws_handshake_rejects_cross_origin`
- `tests/g3_auth.rs::ws_handshake_allows_localhost_origins_including_ipv6_loopback`
- `tests/settings_api.rs::config_set_auth_token_rejected`

### config

**Session·C13** 全局配置 DB：迁移/seed/修复：迁移 v0→current 幂等且只增不删，错误 user_version 失败关闭；seed 种默认 agent 并修复违规行。（守护 7）
- `src/config/global_db/migrate.rs::config_global_db_migration_v0_to_current`
- `src/config/global_db/migrate.rs::re_running_the_migration_is_a_noop`
- `src/config/global_db/migrate.rs::config_global_db_wrong_user_version_fails_closed`
- `src/config/global_db/seed.rs::seed_plants_default_orchestrator_explore_and_general`
- `src/config/global_db/seed.rs::ensure_prunes_rows_that_violate_role_rules`
- `tests/global_db_seed.rs::config_global_db_migration_and_seed`
- `tests/global_db_seed.rs::legacy_role_rule_rows_are_repaired_on_load`

**Session·C14** Settings 写入：turn 期间阻塞、混合提交回滚、密钥掩码、即时生效：turn 运行时写设置被拒；文件+DB 混合写失败恢复文件；key 掩码返回；provider key 下一 turn 生效。（守护 6）
- `tests/settings_api.rs::settings_write_blocked_during_turn`
- `src/config/settings_writer.rs::commit_mixed_restores_file_when_db_step_fails`
- `src/config/settings_writer.rs::commit_mixed_restores_file_when_validate_fails`
- `tests/settings_api.rs::settings_provider_key_write_bumps_revision_and_masks_secret`
- `tests/settings_api.rs::settings_provider_key_write_is_effective_next_turn`
- `src/config/manager.rs::validate_subagent_cannot_bind_subagent_series`

### context_pipeline

**上下文·C1** 装配 / view：内存工作集必须和 DB fold 一致；pending 尾巴没有 log_seq（守护 3）
- `tests/working_set_identity.rs::append_rows_reader_working_set_matches_full_fold`
- `tests/working_set_identity.rs::pipeline_begin_turn_pending_tail_has_no_log_seq`
- `tests/working_set_identity.rs::commit_applied_window_matches_fold`

**上下文·C2** view 补齐未答 call：只在 LLM 视图里补，不落盘（守护 2）
- `tests/stage_a_ctx_consistency.rs::unanswered_calls_pad_llm_view_only_not_disk`
- `src/context_pipeline/mod.rs::padding_alignment_uses_order_not_provider_ids`

**上下文·C3** budget：优先用 provider bill，前缀之外的后缀才估算；compact 后 bill 作废（守护 3）
- `src/context_pipeline/budget.rs::provider_bill_covers_its_prefix_and_estimates_only_the_suffix`
- `src/context_pipeline/budget.rs::rewritten_view_does_not_reuse_a_provider_bill`
- `src/context_pipeline/budget.rs::compact_retires_the_in_memory_bill_and_the_stored_one`

**上下文·C4** 触发 auto compact：阈值按真实用量判断，低 bill 不会因为本地估算偏高而误触发（守护 3）
- `src/context_pipeline/compact.rs::low_provider_bill_does_not_compact_a_hot_local_estimate`
- `src/context_pipeline/compact.rs::provider_bill_over_threshold_compacts_then_the_bill_stays_retired`
- `tests/stage_a_ctx_consistency.rs::real_usage_feeds_compaction_decision_input`

**上下文·C5** manual compact 门槛：30% 含边界；manual 绕过 auto 阈值（守护 2）
- `src/context_pipeline/budget.rs::manual_compact_gate_is_inclusive_at_thirty_percent`
- `tests/stage_a_ctx_consistency.rs::manual_compact_bypasses_auto_threshold_and_preserves_full_history`

**上下文·C6** keep_recent 切点：不拆开 tool 对和 assistant 段，切完放得进 history budget；超过硬上限仍强制处理（守护 4）
- `src/context_pipeline/keep_recent.rs::tool_pair_not_split`
- `src/context_pipeline/keep_recent.rs::assistant_segment_not_split_between_reasoning_and_message`
- `src/context_pipeline/keep_recent.rs::compact_split_fits_under_history_budget`
- `tests/stage_a_ctx_consistency.rs::keep_recent_skip_still_enforces_hard_limit_when_over_budget`

**上下文·C7** compact 只吃已持久化的前缀，保留未提交尾巴；重建时 call_id 不重复（守护 3）
- `tests/stage_a_ctx_consistency.rs::compact_eats_only_persisted_prefix_and_keeps_uncommitted_tail`
- `tests/stage_a_ctx_consistency.rs::compact_rebuild_does_not_repeat_kept_call_ids`
- `tests/working_set_identity.rs::keep_recent_compact_reader_matches_fold_checkpoint_then_kept`

**上下文·C8** compact 请求：超时、取消、关 thinking、用调用方给的 cap；只送纯文本（剥掉 media 和 reasoning）（守护 4）
- `src/context_pipeline/compact.rs::compact_request_times_out_on_silent_provider`
- `src/context_pipeline/compact.rs::compact_request_observes_cancel_while_waiting`
- `src/context_pipeline/compact.rs::compact_request_disables_thinking_and_uses_caller_cap`
- `src/context_pipeline/keep_recent.rs::serialize_strips_media_before_the_compact_prompt`

**上下文·C9** compact 失败回滚：transcript/工作集恢复到快照，lease 释放后可重试，失败后 turn 停止且不再调模型（守护 5）
- `tests/compact_failure_calibers.rs::compact_failure_restores_transcript_snapshot`
- `tests/compact_failure_calibers.rs::auto_compact_failure_restores_pipeline_working_set`
- `tests/compact_failure_calibers.rs::manual_compact_failure_releases_lease_for_retry`
- `tests/compact_failure_calibers.rs::reconnect_then_auto_compact_failure_stops_turn`
- `tests/manual_compact_e2e.rs::manual_compact_retry_after_failure_completes`

**上下文·C10** compact 已持久化但 reload 失败：以 DB 为准，报 Succeeded（守护 1）
- `src/context_pipeline/compact.rs::compact_persist_ok_reload_err_keeps_db_truth`

**上下文·C11** summary / rebased：summary 是 assistant 消息；比 summary 早的 kept 行要 rebase；compact 后续写能接上（守护 4）
- `src/context_pipeline/summary.rs::compact_summary_is_assistant_message_not_user`
- `src/context_pipeline/summary.rs::kept_rows_older_than_the_summary_are_rebased`
- `src/context_pipeline/keep_recent.rs::update_prompt_when_leading_summary`
- `tests/stage_a_ctx_consistency.rs::compact_then_new_step_persists_and_resumes_full_content`

**上下文·C12** media_budget：工具 media 被模型看过一次就丢；用户 media 留到本 turn 结束；被剥掉的 batch 只 rebase 它的应答（守护 5）
- `src/context_pipeline/media_budget.rs::tool_media_is_dropped_once_the_model_has_seen_it`
- `src/context_pipeline/media_budget.rs::latest_tool_batch_keeps_every_media_part`
- `src/context_pipeline/media_budget.rs::user_media_from_an_earlier_turn_is_dropped`
- `src/context_pipeline/media_budget.rs::only_the_answer_to_a_stripped_batch_is_rebased`
- `src/context_pipeline/media_budget.rs::summary_strip_removes_every_media_part`

**上下文·C15** system/env 装配：AGENTS.md 和 CLAUDE.md 的拼接顺序；hidden agent 不拼 md（守护 3）
- `src/context_pipeline/system.rs::primary_splices_agents_md_then_claude_md`
- `src/context_pipeline/system.rs::hidden_never_splices_md`
- `src/context_pipeline/env.rs::build_context_copies_process_snapshot_and_ignores_disk`

**上下文·C23** revert 与上下文一致：revert 后提交会丢弃未提交 delta，不重放也不补齐（守护 3）
- `tests/stage_a_ctx_consistency.rs::revert_contract_three_states`
- `tests/stage_a_ctx_consistency.rs::commit_after_revert_discards_uncommitted_delta`
- `tests/working_set_identity.rs::revert_then_commit_discard_returns_fold_window_and_does_not_append`

### reminder

**上下文·C13** reminder 注入：compact 后 restore 只触发一次；reminder 挂在 checkpoint 上，不额外插 user detail；env 快照变化才再次发出（守护 5）
- `src/reminder/engine.rs::restore_fires_once_after_compaction`
- `tests/stage_a_ctx_consistency.rs::compact_reminder_rides_on_checkpoint_not_extra_user_detail`
- `tests/stage_a_ctx_consistency.rs::compact_keeps_pending_tail_after_post_compact_reminder`
- `src/reminder/sources/tests.rs::tasks_restore_only_after_compaction_and_once`
- `src/reminder/sources/tests.rs::env_emits_once_until_snapshot_changes`

**上下文·C14** mentions reminder：user 行后面跟一条隐藏 reminder，revert 时两条一起删；symbol 没变就不重复附（守护 2）
- `src/reminder/mentions.rs::user_row_is_followed_by_a_hidden_mentions_reminder_and_revert_drops_both`
- `src/reminder/mentions.rs::unchanged_symbol_is_not_attached_again_until_the_body_changes`

### engines/session_search

**上下文·C16** session_search 索引：stale 索引在查询前被追上；索引坏了要报错而不是返回空；增量结果等于重建（守护 4）
- `src/engines/session_search/mod.rs::a_stale_index_is_caught_up_behind_the_query`
- `src/engines/session_search/mod.rs::a_broken_index_is_an_error_not_an_empty_result`
- `src/engines/session_search/dense_parity.rs::an_appended_dense_index_equals_a_clean_rebuild`
- `src/engines/session_search/semantic_index.rs::wp135_index_error_is_failed_index_failed_and_clears_on_success`

**上下文·C17** session_search 检索/呈现：能命中种子文本，scope 和 exclude 生效；超出预算的部分溢出到命名文件；hit 行与虚拟 read 对齐（守护 4，另引用 1）
- `src/engines/session_search/mod.rs::session_text_search_finds_seeded_transcript`
- `src/engines/session_search/mod.rs::context_window_exclude_drops_live_seqs_keeps_archive`
- `src/engines/session_search/mod.rs::agent_view_spills_the_remainder_into_a_named_file`
- `src/tools/session_search.rs::compacted_history_below_kept_from_seq_remains_searchable`
- `src/tools/session_search.rs::search_hit_line_matches_virtual_read` → 归 Tools·C18

### engines/code_search

**上下文·C18** code_search 索引持久化：多文件原子写，崩溃后可恢复；commit 和 stamps 之间崩溃不会漏（守护 3）
- `src/engines/code_search/atomic.rs::crash_between_renames_leaves_prefix_and_sweep_clears_tmp`
- `src/engines/code_search/mod.rs::crash_between_compaction_renames_keeps_chunks_and_vectors_one_to_one`
- `src/engines/code_search/reconcile.rs::crash_between_index_commit_and_stamps_rereads_never_misses`

**上下文·C19** code_search IPC：worker 全生命周期；fs 通知后能搜到新文件；meta 不兼容才重建，其余情况走增量（守护 4）
- `tests/code_search_ipc.rs::worker_ping_initialize_warmup_search_shutdown`
- `tests/code_search_ipc.rs::notify_fs_changes_then_search_hits_new_file`
- `tests/code_search_ipc.rs::refresh_incremental_then_rebuild_when_meta_incompatible`
- `tests/code_search_ipc.rs::refresh_stays_incremental_when_all_files_dirty`

**上下文·C20** code_search 可选降级：engine 关掉或 worker 崩溃时降级不卡；warming 期间工具已列出；停止时不留僵尸（守护 5）
- `tests/optional_code_search.rs::worker_crash_demotes_engine`
- `tests/optional_code_search.rs::engine_desired_off_stops_engine`
- `tests/optional_code_search.rs::catalog_lists_tool_while_warming_and_after_warm`
- `tests/optional_code_search.rs::stop_during_warmup_leaves_no_zombie_worker`
- `tests/optional_code_search.rs::warmup_loads_compatible_index_without_full_rebuild`

**上下文·C21** code_index tick：tick 只做同步，不 build 也不 cool；手动 refresh 不合并 plan（守护 4）
- `tests/code_index_tick_death_list.rs::code_index_tick_never_builds_or_cools`
- `tests/code_index_tick_death_list.rs::manual_refresh_does_not_merge_plans_for_execution`
- `tests/optional_code_search.rs::code_index_tick_syncs_without_refresh`
- `src/engines/code_search/mod.rs::pending_blocks_cool_drop`

### knowledge

**上下文·C22** knowledge 加载：增量结果等于从零重建；turn 只读快照；状态分类（守护 5）
- `src/knowledge/index_tests.rs::incremental_equals_from_scratch_over_random_edits`
- `src/knowledge/index_tests.rs::the_turn_reads_the_snapshot_only`
- `src/knowledge/index_tests.rs::every_file_is_parsed_once_per_stamp`
- `src/knowledge/spec_tests.rs::resolution_follows_the_spec`
- `src/knowledge/status.rs::classes_cover_errors_warnings_and_enabled`

### llm

**Provider·P1** SSE line reader (`src/llm/codec/sse.rs`)：Framed SSE bytes are decoded into complete `data:` lines without corrupting UTF-8 across chunk boundaries, with BOM/CRLF handling and hard limits on noise and line size.（守护 6）
- `src/llm/codec/sse.rs::multibyte_char_split_across_chunks_is_preserved`
- `src/llm/codec/sse.rs::leading_bom_is_stripped_once`
- `src/llm/codec/sse.rs::crlf_lines_strip_cr`
- `src/llm/codec/sse.rs::skips_noise`
- `src/llm/codec/sse.rs::trailing_line_without_newline_is_kept_for_finish`
- `src/llm/codec/sse.rs::oversized_line_is_a_hard_error`

**Provider·P2** Stream contract — function-call event gate (`stream_contract.rs`)：Out-of-order Responses tool-call events are normalized (synthetic `output_item.added`, name patches, seq ordering) before the accumulator runs.（守护 5）
- `src/llm/codec/stream_contract.rs::delta_before_added_synthesizes_added_first`
- `src/llm/codec/stream_contract.rs::empty_name_then_done_with_name_emits_patch_added`
- `src/llm/codec/stream_contract.rs::real_output_index_zero_is_preserved_not_treated_as_unknown`
- `src/llm/codec/stream_contract.rs::synthesized_seqs_stay_above_all_provider_seqs`
- `src/llm/codec/stream_contract.rs::empty_item_id_delta_does_not_synthesize_added`

**Provider·P3** Stream contract — accumulator, terminals, cancel (`stream_contract.rs`)：Live deltas become authority `Item`s; `response.completed` / `response.incomplete` / `response.failed` and user cancel resolve to items, errors, or `Canceled` without duplicating ids.（守护 8）
- `src/llm/codec/stream_contract.rs::incomplete_terminal_yields_items`
- `src/llm/codec/stream_contract.rs::failed_terminal_still_errors`
- `src/llm/codec/stream_contract.rs::text_delta_seal_incomplete_without_terminal`
- `src/llm/codec/stream_contract.rs::resolve_empty_cancel_is_canceled`
- `src/llm/codec/stream_contract.rs::output_text_done_replaces_streamed_text`
- `src/llm/codec/stream_contract.rs::output_item_done_replaces_the_streamed_shell`
- `src/llm/codec/stream_contract.rs::duplicate_item_id_in_completed_output_is_refused`
- `src/llm/codec/stream_contract.rs::real_stream_shape_normalizes_to_one_item_per_id_in_order`

**Provider·P4** HTTP send — retry, cancel, empty-stream give-up (`src/llm/codec/http.rs`)：`send_cancellable` honors cancel, retries only retryable pre-stream failures, and surfaces reconnect failure without treating user cancel as failed.（守护 6）
- `src/llm/codec/http.rs::send_cancellable_returns_canceled_when_token_fires`
- `src/llm/codec/http.rs::send_cancellable_retries_transient_status_before_stream_starts`
- `src/llm/codec/http.rs::send_cancellable_does_not_retry_explicit_client_failure`
- `src/llm/codec/http.rs::cancel_while_waiting_does_not_emit_failed`
- `src/llm/codec/http.rs::empty_stream_loss_on_a_spent_budget_emits_failed`
- `src/llm/codec/http.rs::retry_emits_waiting_then_connecting_then_cleared`

**Provider·P5** Reconnect UI — compaction turn (`src/llm/reconnect.rs`)：A terminal reconnect `Failed` during compaction is cleared so the UI does not stay stuck in failed state.（守护 1）
- `src/llm/reconnect.rs::compaction_scope_turns_a_terminal_failure_into_cleared`

**Provider·P6** Responses codec — streaming product path (`src/llm/codec/responses.rs`)：Responses SSE is forwarded to `on_event`; final `Item`s come from the terminal payload; cancel/disconnect/proxy misconfiguration behave as specified.（守护 7）
- `src/llm/codec/responses.rs::stream_events_are_forwarded_and_items_come_from_the_terminal`
- `src/llm/codec/responses.rs::keepalive_between_events_does_not_abort_the_stream`
- `src/llm/codec/responses.rs::cancel_after_a_delta_seals_incomplete`
- `src/llm/codec/responses.rs::cancel_before_any_event_is_canceled`
- `src/llm/codec/responses.rs::transport_disconnect_returns_partial_items_with_an_explicit_error`
- `src/llm/codec/responses.rs::truncated_sse_after_items_is_terminal_and_keeps_partials`
- `src/llm/codec/responses.rs::non_event_stream_content_type_surfaces_the_proxy_body`

**Provider·P7** Responses codec — request encode & reasoning replay (`responses.rs`)：Wire body matches catalog capabilities; foreign/own reasoning ciphertext and summary replay follow quirk rules.（守护 3）
- `src/llm/codec/responses.rs::plain_model_sends_the_authority_shape`
- `src/llm/codec/responses.rs::undeclared_dialect_replays_only_its_own_ciphertext`
- `src/llm/codec/responses.rs::summary_replay_sends_own_id_and_summary_text`

**Provider·P8** Responses stream hardening (`src/llm/codec/responses_harden.rs`)：Vendor/tool-call JSON shapes are coerced so downstream parsing sees string arguments and patched usage fields where configured.（守护 3）
- `src/llm/codec/responses_harden.rs::tool_call_object_arguments_become_a_json_string`
- `src/llm/codec/responses_harden.rs::tool_call_string_arguments_stay_verbatim`
- `src/llm/codec/responses_harden.rs::deepseek_patch_also_fills_total_and_effort`

**Provider·P9** Messages codec — wire encode & tool pairing (`messages.rs`)：Anthropic-shaped bodies preserve prefix continuity, repair missing tool results, and round-trip thinking signatures inside an open tool turn.（守护 3）
- `src/llm/codec/messages.rs::later_turn_extends_the_previous_messages_prefix`
- `src/llm/codec/messages.rs::a_missing_tool_result_is_repaired`
- `src/llm/codec/messages.rs::thinking_and_tool_signatures_round_trip_inside_the_open_turn`

**Provider·P10** Messages codec — streaming (`messages.rs`)：Messages SSE completes with signature preserved; retries/cancel/disconnect/error timing match Chat/Responses policy.（守护 5）
- `src/llm/codec/messages.rs::stream_round_trip_keeps_the_signature_and_sends_the_version_header`
- `src/llm/codec/messages.rs::empty_stream_retries_until_a_terminal_arrives`
- `src/llm/codec/messages.rs::stream_error_before_items_retries_and_after_items_preserves`
- `src/llm/codec/messages.rs::disconnect_after_items_keeps_the_partial`
- `src/llm/codec/messages.rs::cancel_after_a_delta_seals_incomplete`

**Provider·P11** Messages SSE → Responses events synth (`messages_synth.rs`)：Anthropic stream events are synthesized into Responses-shaped stream events with stable parallel tool ids and dropped-thinking accounting.（守护 3）
- `src/llm/codec/messages_synth.rs::message_start_records_dropped_thinking_without_changing_the_stream`
- `src/llm/codec/messages_synth.rs::parallel_tool_calls_keep_their_own_ids`
- `src/llm/codec/messages_synth.rs::thinking_text_and_tool_round_trip_with_the_signature`

**Provider·P12** Chat completions codec — encode, replay, stream (`chat.rs`)：Chat wire encoding replays reasoning into assistant turns; streaming emits early tool `added`, usage on terminal, and skips whitespace-only assistant shells.（守护 6）
- `src/llm/codec/chat.rs::tools_and_reasoning_replay_into_one_assistant_turn`
- `src/llm/codec/chat.rs::recorded_reasoning_is_replayed_as_reasoning_content`
- `src/llm/codec/chat.rs::stream_tool_call_opens_the_item_before_the_arguments_delta`
- `src/llm/codec/chat.rs::stream_usage_lands_on_the_terminal_response`
- `src/llm/codec/chat.rs::stream_whitespace_only_content_leaves_no_message_item`
- `src/llm/codec/chat.rs::stream_text_only_becomes_an_assistant_message`

**Provider·P13** Chat SSE synth (`chat_synth.rs`)：Chat completion deltas that are whitespace-only must not create a message item until visible content arrives.（守护 1）
- `src/llm/codec/chat_synth.rs::whitespace_only_content_never_births_a_message`

**Provider·P14** Cross-item replay compatibility (`replay_compat.rs`)：Reasoning ciphertext/signatures are stripped or retained based on producer provider, endpoint kind, model ref, and prefix rewrite — text stays on the item when Responses replay allows.（守护 5）
- `src/llm/replay_compat.rs::ciphertext_goes_back_only_to_its_producer_and_text_always_stays`
- `src/llm/replay_compat.rs::summary_replay_keeps_only_this_providers_reasoning`
- `src/llm/replay_compat.rs::ciphertext_does_not_cross_protocols_on_the_same_provider`
- `src/llm/replay_compat.rs::messages_signature_does_not_survive_a_rewritten_prefix`
- `src/llm/replay_compat.rs::producer_is_the_newest_header_at_or_before_the_item`

### provider_catalog

**Provider·P15** Provider catalog — load, resolve, strict validation (`provider_catalog/tests.rs`)：Embedded seed is the sole catalog source; models resolve to URLs by endpoint kind; invalid TOML is rejected, not trimmed.（守护 6）
- `src/provider_catalog/tests.rs::embedded_catalog_is_valid_and_complete`
- `src/provider_catalog/tests.rs::shared_catalog_is_the_embedded_seed_and_does_not_read_a_file`
- `src/provider_catalog/tests.rs::references_are_provider_slash_model_and_lookup_splits_once`
- `src/provider_catalog/tests.rs::request_url_comes_from_endpoint_type`
- `src/provider_catalog/tests.rs::unknown_field_is_refused`
- `src/provider_catalog/tests.rs::codec_capability_mismatch_is_refused_not_trimmed`

### 集成（provider 产品路径）

**Provider·P16** Agent e2e — Responses codec on product runtime (`tests/agent_e2e_responses.rs`)：Real runtime turn against local Responses SSE: text-only, tool loop, incomplete seal to transcript.（守护 3）
- `tests/agent_e2e_responses.rs::agent_e2e_responses_text_only`
- `tests/agent_e2e_responses.rs::agent_e2e_responses_tool_loop_hits_max_steps`
- `tests/agent_e2e_responses.rs::agent_e2e_incomplete_seals_text_to_disk`

**Provider·P17** Turn prefix stability — prompt-cache identity (`tests/turn_prefix_stability.rs`)：Within a turn (and across reload with reused provider ids), already-sent `ModelRequest` JSON prefix must not be rewritten by persist/stream/seal.（守护 1，另引用 4）
- `tests/turn_prefix_stability.rs::prepare_after_stream_persist_seal_keeps_prior_prefix`
- `tests/turn_prefix_stability.rs::tool_loop_llm_request_prefix_json_is_stable` → 归 Runtime·C10
- `tests/turn_prefix_stability.rs::persist_item_reused_function_call_id_must_not_rewrite_final_row` → 归 Runtime·C10
- `tests/turn_prefix_stability.rs::persist_item_reused_message_id_must_not_rewrite_final_row` → 归 Runtime·C10
- `tests/turn_prefix_stability.rs::next_turn_reload_must_keep_sent_row_when_provider_reuses_id` → 归 Runtime·C10

### session/snapshot

**Session·C15** snapshot 文件回滚（issue_032，GM 定为核心）：只恢复 patch 涉及的路径、warm 后增量编辑可回滚、缺 track ref 报 unavailable；spine 失败后下一 seam 恢复已改文件（守护 4）
- `src/session/snapshot.rs::file_level_restore_only_touches_patch_paths`
- `src/session/snapshot.rs::warm_then_incremental_edit_restore`
- `src/session/snapshot.rs::restore_missing_track_ref_is_unavailable`
- `tests/issue_032_changed_files_restore.rs::t1_spine_failure_restores_changed_files_for_next_seam`

### workspace

**IDE·L1** Agent/LSP/人工 API 路径解析边界：相对路径带 `..` 时所有模式都拒绝；Safe 和 LSP 拒绝工作区外的绝对路径，All 放行；人工 API 不接受绝对路径。（守护 7）
- `src/workspace/tool_path.rs::relative_traversal_is_rejected_in_all_modes`
- `src/workspace/tool_path.rs::all_allows_and_safe_rejects_external_absolute_path`
- `src/workspace/tool_path.rs::unix_absolute_outside_workspace_safe_rejected`
- `src/workspace/tool_path.rs::human_relative_rejects_absolute_paths`
- `src/workspace/tool_path.rs::agent_and_lsp_share_safe_scope`
- `src/workspace/tool_path.rs::raw_path_outside_workspace_matches_resolved_boundary`
- `src/workspace/agent_path.rs::relative_traversal_is_rejected`

**IDE·L2** symlink 逃逸（目标文件还不存在时）：Safe 模式按"最深的已存在祖先"做 canonicalize，经 symlink 目录写新文件一律拒绝。（守护 3）
- `src/workspace/tool_path.rs::symlinked_dir_escape_rejected_for_new_file`
- `src/workspace/tool_path.rs::symlinked_subdir_escape_rejected_for_new_file`
- `src/workspace/tool_path.rs::symlinked_dir_escape_rejected_for_new_file_windows`

**IDE·L3** Windows 下的 Unix 风格路径：`/etc/passwd` 不能被误判成相对路径或 traversal，要么映射到 Git for Windows 的路径，要么明确报错。（守护 3）
- `src/workspace/agent_path.rs::unix_style_on_windows_not_traversal`
- `src/workspace/tool_path.rs::unix_style_passwd_not_relative_traversal`
- `src/workspace/agent_path.rs::msys_drive_mapping`

**IDE·L4** 人工 HTTP 和 git 接口越界返回 403：Sandbox::resolve 只要遇到 traversal、外部或 Unix 风格路径，统一返回 Escape，HTTP 层映射成 403。（守护 6）
- `src/workspace/sandbox.rs::rejects_parent_dir_escape`
- `tests/workspace_api.rs::workspace_rejects_path_escape`
- `tests/workspace_api.rs::workspace_git_stage_commit_and_rejects_escape`
- `tests/workspace_api.rs::workspace_git_show_head_index_and_rejects`
- `tests/workspace_api.rs::workspace_preview_reads_and_open_guards`
- `src/workspace/git.rs::path_escape_is_rejected`

**IDE·L5** 复制 symlink 不外泄：复制或保留 symlink 时，被指向的对象要过沙箱；指向外部的 symlink 拒绝复制，也不留半成品。（守护 3）
- `src/workspace/service.rs::refuses_symlink_copy_escaping_workspace`
- `src/workspace/service.rs::preserves_symlink_top_level`
- `src/workspace/service.rs::preserves_symlink_nested`

**IDE·L6** 引用/预览不泄露文件内容，越界引用被隐藏（守护 3）
- `src/workspace/citations.rs::resolve_reports_exists_line_symbol_and_hides_escapes`
- `tests/workspace_api.rs::workspace_citations_resolve_without_file_bodies`
- `src/workspace/sqlite_preview.rs::rejects_unknown_and_injected_table_names`

**IDE·L7** LAP 统一形态（verbatim、UNC、canonicalize 白名单）：所有根路径和比较都用 LAP 形态；canonicalize 只允许出现在 config/path.rs。（守护 18）
- `tests/workspace_path_death_list.rs::death_list_no_bare_canonicalize_outside_path_module`
- `tests/workspace_path_death_list.rs::death_list_src_canonicalize_whitelist_is_only_path_rs`
- `tests/workspace_path_death_list.rs::death_list_web_no_silent_verbatim_strip`
- `tests/workspace_path_lap.rs::boot_sandbox_lsp_os_probe_roots_are_identical_lap`
- `tests/workspace_path_lap.rs::strip_verbatim_unc_keeps_unc_prefix`
- `tests/workspace_path_lap.rs::windows_raw_canonicalize_is_verbatim_then_strips_to_lap`
- `tests/workspace_path_lap.rs::snapshot_id_stable_across_verbatim_and_lap_input`
- `tests/workspace_path_lap.rs::rel_path_ctx_matches_rel_path_under_and_rejects_outside`
- `src/workspace/filter/path.rs::rel_path_ctx_rejects_outside`
- `src/workspace/filter/path.rs::cheap_rel_under_rejects_outside`
- `src/workspace/filter/path.rs::cheap_rel_under_strips_verbatim_prefix`
- `tests/workspace_path_consumer.rs::rel_path_under_accepts_raw_canonicalize_form`
- `tests/workspace_path_consumer.rs::windows_verbatim_walk_path_relativizes`
- `src/workspace/sandbox.rs::root_is_lap_not_verbatim`
- `src/config/path.rs::canon_abs_resolves_symlink`
- `src/config/path.rs::strip_uppercases_drive_letter`
- `src/config/path.rs::strip_verbatim_drive_and_unc`
- `src/config/path.rs::is_under_is_case_insensitive_on_windows`

**IDE·L16** 原子写：写入时保留 symlink 节点、权限位和硬链接；create-only 模式不覆盖已有文件。（守护 5）
- `src/workspace/file_ops.rs::atomic_write_writes_through_symlink`
- `src/workspace/file_ops.rs::atomic_write_preserves_execute_bit_on_overwrite`
- `src/workspace/file_ops.rs::atomic_write_preserves_hardlink_inode`
- `src/workspace/file_ops.rs::atomic_create_fails_if_exists_without_clobber`
- `src/workspace/file_ops.rs::atomic_write_creates_file_with_content`

**IDE·L17** 工作区进程契约：不 chdir；同一个根只能加一把锁；agent 的 cwd 取已解析的工作区。（守护 4）
- `tests/workspace_process_contract.rs::load_workspace_state_sets_runtime_paths_without_chdir`
- `tests/workspace_process_contract.rs::workspace_lock_second_acquire_same_root_fails`
- `tests/workspace_process_contract.rs::agent_runtime_cwd_uses_resolved_workspace_not_process_cwd`
- `tests/workspace_path_consumer.rs::active_paths_fallback_is_lap`

**IDE·L18** watcher 与 git 环境隔离（守护 6）
- `src/workspace/watcher.rs::paths_outside_root_are_ignored`
- `src/workspace/watcher.rs::atomic_save_overwrite_is_reclassified_as_modified`
- `src/workspace/watcher.rs::rescan_flag_and_overflow_request_full_rescan`
- `src/workspace/git.rs::inherited_git_dir_env_does_not_select_snapshot_repo`
- `src/workspace/git.rs::git_dir_is_snapshot_detects_shadow_repo`
- `src/workspace/git.rs::git_commands_skip_optional_index_locks`

### lsp

**IDE·L8** LSP 输出路径：工作区内的路径显示为相对路径，工作区外保留绝对路径；display_path 只做文本前缀剥离，不解析 symlink。（守护 5）
- `src/lsp/format.rs::location_inside_workspace_is_relative`
- `src/lsp/format.rs::location_outside_workspace_stays_absolute`
- `src/lsp/format.rs::workspace_symbol_paths_relative_inside_absolute_outside`
- `src/lsp/format.rs::display_path_does_not_resolve_symlinks`
- `src/lsp/format.rs::display_path_tries_roots_in_order`

**IDE·L9** LSP URI 与盘符：file URI 去掉 verbatim 前缀、盘符不区分大小写、能还原被破坏的 verbatim 形态。（守护 5）
- `src/lsp/mod.rs::file_to_uri_strips_verbatim_prefix`
- `src/lsp/mod.rs::uri_to_path_recovers_mangled_verbatim_file_uri`
- `src/lsp/mod.rs::normalize_windows_verbatim_file_uri`
- `src/lsp/mod.rs::publish_diagnostics_uri_matches_drive_case`
- `src/lsp/mod.rs::publish_diagnostics_uri_matches_mangled_verbatim`

**IDE·L10** 诊断时效（不回显过时的错误）（守护 13）
- `src/lsp/conn.rs::diagnostics_are_current_rejects_stale_cache`
- `src/lsp/conn.rs::cache_covers_sync_requires_matching_version`
- `src/lsp/conn.rs::wait_ignores_stale_publish_and_keeps_waiting_for_current`
- `src/lsp/conn.rs::wait_returns_current_errors_without_blocking`
- `src/lsp/conn.rs::wait_returns_current_clean_cache_without_blocking`
- `src/lsp/conn.rs::unversioned_publish_after_change_is_silence_not_stale_errors`
- `src/lsp/conn.rs::edit_fix_must_not_echo_previous_round_error`
- `tests/optional_lsp.rs::write_without_lsp_warm_never_appends_diagnostics`
- `tests/optional_lsp.rs::write_with_warm_lsp_but_no_errors_stays_clean`
- `tests/optional_lsp.rs::write_with_warm_lsp_appends_only_when_errors_arrive`
- `tests/optional_lsp.rs::write_ignores_slow_diagnostics_past_budget`
- `tests/optional_lsp.rs::write_ignores_warn_only_diagnostics`
- `tests/optional_lsp.rs::edit_fix_must_not_echo_previous_lsp_error`

**IDE·L11** ContentModified 处理：只认错误码 -32801，最多取消一次；透传型 RPC 保留原始错误。（守护 5）
- `src/lsp/conn.rs::is_content_modified_matches_only_32801`
- `src/lsp/conn.rs::content_modified_plain_request_returns_empty_once`
- `src/lsp/conn.rs::content_modified_semantic_tokens_is_cancelled_once`
- `src/lsp/conn.rs::content_modified_resolve_is_cancelled_once`
- `src/lsp/conn.rs::content_modified_passthrough_rpc_id_keeps_raw_error`

**IDE·L12** LSP 进程生命周期：先 sweep 再 spawn；hub 被 drop 时杀掉子进程；请求超时不杀进程；重启有预算。（守护 11）
- `src/lsp/mod.rs::spawn_ls_waits_for_sweep_gate`
- `tests/optional_lsp.rs::hub_drop_kills_spawned_server_process`
- `tests/optional_lsp.rs::request_timeout_does_not_kill_process`
- `tests/optional_lsp.rs::idle_reclaim_during_diagnostics_does_not_panic`
- `src/lsp/hub.rs::restart_budget_resets_after_cooldown`
- `src/lsp/conn.rs::dispatch_out_of_order_responses`
- `tests/optional_lsp.rs::overlapping_hovers_match_out_of_order_ids`
- `src/lsp/sweep_gate.rs::wait_passes_when_never_armed`
- `src/lsp/sweep_gate.rs::wait_blocks_until_sweep_returns`
- `src/lsp/sweep_gate.rs::gate_opens_when_sweep_panics`
- `src/lsp/sweep_gate.rs::second_start_is_noop_after_open`

**IDE·L13** 冷启动与 hub 激活：probe 推迟到第一次真正需要时才跑；probe 失败会让 hub 失活并报出原始错误；索引未就绪时报 unavailable。（守护 8）
- `src/lsp/mod.rs::deferred_activate_runs_no_probe_until_first_need`
- `src/lsp/mod.rs::probe_runs_once_per_activation`
- `src/lsp/mod.rs::probe_failure_deactivates_hub_and_reports_original_error`
- `src/lsp/mod.rs::restart_after_failed_probe_says_hub_not_active`
- `src/lsp/mod.rs::coverage_requires_active_and_configured_command`
- `src/lsp/server.rs::index_not_ready_error_maps_to_unavailable_wording`
- `src/lsp/server.rs::diagnostics_gate_table`
- `src/lsp/server.rs::nav_empty_symbols_by_array_len`

**IDE·L14** 项目根选择（守护 4）
- `src/lsp/project_root.rs::rust_analyzer_uses_cargo_workspace_not_member_crate`
- `src/lsp/project_root.rs::rust_nested_crate_outside_workspace_members_keeps_own_root`
- `src/lsp/project_root.rs::monorepo_ts_file_resolves_to_web_subdir`
- `tests/workspace_path_consumer.rs::project_root_fallback_accepts_verbatim_under_lap_ws`

### optional

**IDE·L15** 可选引擎懒构建与 reconcile（守护 10）
- `src/optional/mod.rs::defer_reconcile_builds_no_client`
- `src/optional/mod.rs::ensure_warm_builds_once_and_reuses`
- `src/optional/mod.rs::concurrent_first_calls_build_once`
- `src/optional/mod.rs::failed_build_is_not_cached_and_retries`
- `src/optional/mod.rs::stopped_engine_is_not_rebuilt`
- `src/optional/mod.rs::settings_reconcile_rebuilds_immediately`
- `src/optional/mod.rs::poisoned_state_lock_does_not_cascade_panics`
- `tests/optional_p0.rs::settings_reload_reconciles_engine_manager`
- `tests/optional_p0.rs::network_core_engines_always_warmup`
- `tests/optional_lsp.rs::engines_json_off_no_lsp_tool`

### 跨域归属说明

- `tests/agent_e2e_responses.rs`：归 Provider（codec 产品路径 e2e）；Runtime C8 引用 `agent_e2e_incomplete_seals_text_to_disk`。
- `tests/turn_prefix_stability.rs`：归 Runtime C10（请求前缀稳定）；Provider「Turn prefix stability」链引用，Provider 额外列的 `prepare_after_stream_persist_seal_keeps_prior_prefix`（Runtime 未列）仍归 Provider。
- `tests/f2_lock_scope.rs`（2 条）：Tools 剔出，归 Session C6（manager 锁 / 不丢更新）。
- `src/config/path.rs`（4 条：canon_abs_resolves_symlink、strip_uppercases_drive_letter、strip_verbatim_drive_and_unc、is_under_is_case_insensitive_on_windows）：IDE 与 Session 互相移交，GM 定归 IDE L7。
- `src/tools/session_search.rs`：归 Tools C18；上下文 C17 引用 `search_hit_line_matches_virtual_read`；`compacted_history_below_kept_from_seq_remains_searchable` 被 Tools 剔为 session 行为，由上下文 C17 守护。`src/engines/session_search/*` 归上下文。

## 2. 回归测试（带出处，跨域合并）

- `src/permission/grants.rs::wp081_legacy_q0_free_text_always_true` — Tools：WP-081（ask_user 自由文本恒在）
- `src/tool/registry.rs::core_bindings_produce_full_builtin_list` — Tools：WP-197（默认绑定无旧 shell，有 run/wait/kill）、WP-057（session_search 门）
- `src/tools/code_search.rs::indexing_wait_message_reads_job_json` — Tools：WP-143
- `src/tools/code_search.rs::warm_stale_is_searchable_without_warm_wait` — Tools：WP-057/060/061
- `src/tools/custom.rs::child_ignoring_stdin_is_not_a_broken_pipe_failure` — Tools：WP-018（Broken pipe）
- `src/tools/custom.rs::finished_reaps_grandchild_holding_stdout` — Tools：ISSUE-039 X1/X2
- `src/tools/custom.rs::finished_kills_orphaned_grandchild_before_side_effect` — Tools：ISSUE-039 X2
- `src/tools/lsp.rs::index_not_ready_rejects_unavailable_for_symbols_and_diagnostics` — Tools：WP-206 ③
- `src/tools/lsp.rs::finish_locations_relative_inside_absolute_outside` — Tools：WP-206 ⑧
- `src/tools/code/render.rs::dropped_undefined_line_sits_in_the_head` — Tools：WP-206 #7（实现侧注释，测试名对应）
- `src/tools/read.rs::test_read_start_line_past_end_resolves_empty` — Tools：WP-206 ④（commit 1350f56）
- `src/tools/read.rs::test_read_empty_file_past_end_resolves_empty` — Tools：WP-206 ④（commit 1350f56）
- `src/tools/plan.rs::test_create_rejected_does_not_create_file` — Tools：WP-050 T3
- `src/tools/run.rs::fast_command_returns_exit_code` — Tools：WP-197（仅 echo 标记串 hello-wp197，弱引用）
- `src/tools/run_lock_gate.rs::cancel_before_spawn_releases_when_the_call_ends` — Tools：WP-206 A 组（模块头）
- `src/tools/run_lock_gate.rs::dropped_in_flight_future_releases_and_panic_unwind_releases` — Tools：WP-206 A 组
- `src/tools/run_lock_gate.rs::four_parallel_write_runs_finish_in_about_one_second` — Tools：WP-206 A 组（id wp206-a）
- `src/tools/run_lock_gate.rs::locks_release_after_spawn_while_command_still_runs` — Tools：WP-206 A 组
- `src/tools/run_lock_gate.rs::short_command_returns_exit_code_in_foreground` — Tools：WP-206 A 组
- `src/tools/run_lock_gate.rs::spawn_failure_releases_locks` — Tools：WP-206 A 组
- `src/tools/run_lock_gate.rs::window_is_capped_by_run_code_deadline` — Tools：WP-206 A 组
- `src/tools/subagent/hub.rs::hub_lagged_rescan_dedups_per_child` — Tools：ISSUE-054
- `src/tools/subagent/spawn_contract.rs::child_calls_agent_provider_endpoint_not_parent_provider` — Tools：parent-provider clobber 修复回归（注释明示）
- `src/tools/webfetch.rs::ssrf_rejects_loopback_and_private_ips` — Tools：ISSUE-012（IPv4-mapped）、WP-203（100.64 仍拒）
- `src/tools/webfetch.rs::ssrf_allows_benchmarking_fake_ip_range` — Tools：WP-203（198.18/15 放行）
- `src/tools/webfetch.rs::ssrf_allows_hostname_resolved_to_fake_ip` — Tools：WP-203
- `src/mcp/client.rs::r1_skips_notification_before_response` — Tools：ISSUE-034 R1（实现注释）
- `src/mcp/client.rs::r2_hang_times_out_and_marks_dirty` — Tools：ISSUE-034 R2
- `src/mcp/client.rs::r3_cancel_marks_dirty_next_client_ok` — Tools：ISSUE-034 R3
- `tests/wp028_probe_s1_xdev_move.rs::probe_s1_cross_device_overwrite_succeeds` — Tools：WP-040 / ISSUE-038
- `tests/wp028_probe_s2_create_race.rs::probe_s2_create_file_race` — Tools：WP-040 / ISSUE-037
- `tests/wp028_probe_s2_create_race.rs::probe_s2_write_file_bytes_no_overwrite_race` — Tools：WP-040 / ISSUE-037
- `tests/agent_loop.rs::cancel_during_auto_compact_is_cancelled_without_model_call` — Runtime：ISSUE-033
- `tests/subagent_send.rs::send_to_busy_child_queues_without_starting_a_turn` — Runtime：ISSUE-053, WP-092, WP-100
- `tests/subagent_send.rs::concurrent_sends_only_one_wins` — Runtime：ISSUE-053, WP-092
- `tests/wp098_follow_up_queue.rs::runtime_main_session_pending_still_injects_at_seam` — Runtime：WP-098
- `tests/wp098_follow_up_queue.rs::runtime_steer_still_injects_at_step_seam` — Runtime：WP-098
- `tests/wp098_follow_up_queue.rs::runtime_queue_not_drained_at_step_seam` — Runtime：WP-098
- `tests/wp098_follow_up_queue.rs::runtime_stop_and_send_cancels_then_runs_new_turn_no_notice` — Runtime：WP-098
- `tests/wp098_follow_up_queue.rs::stop_and_send_cancels_then_runs_follow_up_without_waking_parent` — Runtime：WP-098
- `tests/startup_order_death_list.rs::serve_run_does_not_sweep_inline` — Runtime：WP-165
- `src/ptc/tests.rs::host_receives_extra_arguments_as_a_json_array` — Runtime：WP-206
- `src/ptc/tests.rs::undefined_in_result_is_stripped_or_nulled_and_reported` — Runtime：WP-206
- `src/runtime/observer.rs::turn_end_reason_log_schema_is_snake_case_for_every_variant` — Runtime：WP-069
- `src/runtime/observer.rs::turn_error_kind_serializes_snake_case` — Runtime：WP-069
- `src/runtime/exec.rs::body_bytes_reflects_instructions` — Runtime：WP-090
- `src/runtime/pending_auto_turn.rs::child_uses_deny_sink_regardless_of_last_sink` — Runtime：WP-098
- `src/ptc/tests.rs::stack_overflow_caught_then_immediate_return_is_still_sealed` — Runtime：WP-207 G1 / 99ac65f
- `src/ptc/tests.rs::dropped_undefined_list_is_capped_with_more_count` — Runtime：WP-206 D / 61cfd38
- `src/ptc/tests.rs::undefined_in_arguments_is_dropped_silently` — Runtime：WP-206 D / 61cfd38
- `src/ptc/engine.rs::timeout_during_drain_keeps_the_returned_value` — Runtime：drain 保值修复
- `src/ptc/engine.rs::cancel_during_drain_keeps_the_value` — Runtime：drain 保值修复
- `src/client_protocol/connection.rs::g4_compact_while_running_is_agent_already_running_retryable` — Session：WP-024
- `src/client_protocol/connection.rs::unknown_grant_is_buffered_as_stray_then_consumed` — Session：WP-091, wp091
- `src/client_protocol/connection.rs::wp091_agent_cancel_child_writes_parent_notice_no_wake` — Session：wp091
- `src/client_protocol/connection.rs::wp091_agent_cancel_reports_false_when_idle` — Session：wp091
- `src/client_protocol/connection.rs::wp091_agent_run_on_child_rejected_readonly` — Session：wp091
- `src/client_protocol/connection.rs::wp091_buffer_load_returns_subagent_binding_states` — Session：WP-099, wp091, wp099
- `src/client_protocol/connection.rs::wp091_delete_child_tombstone_and_notice` — Session：wp091
- `src/client_protocol/connection.rs::wp091_delete_parent_with_running_child_returns_descendant_running` — Session：wp091
- `src/client_protocol/connection.rs::wp091_delete_running_child_rejected` — Session：wp091
- `src/client_protocol/connection.rs::wp091_enqueue_on_running_child_rejected_readonly` — Session：wp091
- `src/client_protocol/connection.rs::wp091_list_sessions_includes_empty_child` — Session：wp091
- `src/client_protocol/connection.rs::wp091_pending_enqueue_on_child_rejected_readonly` — Session：wp091
- `src/client_protocol/connection.rs::wp091_retry_on_child_rejected_readonly` — Session：wp091
- `src/client_protocol/connection.rs::wp099_child_still_readonly_for_user` — Session：wp091, wp099
- `src/client_protocol/connection.rs::wp099_session_info_last_turn_cancelled_by_user_parent_null` — Session：wp091, wp099
- `src/client_protocol/connection.rs::wp099_user_cancel_child_notice_by_user_and_no_wake` — Session：wp091, wp099
- `src/client_protocol/project.rs::auto_compact_failure_maps_to_compaction_failed_not_retryable` — Session：ISSUE-036, wp025, wp186
- `src/client_protocol/status_track.rs::installed_hook_is_registered` — Session：wp099
- `src/client_protocol/status_track.rs::last_turn_cancelled_by_survives_reload` — Session：wp099
- `src/client_protocol/status_track.rs::turn_end_cancelled_by_user_parent_null` — Session：wp099
- `src/client_protocol/status_track.rs::turn_finished_event_carries_cancelled_by` — Session：wp099
- `src/client_protocol/ws_wp166_tests.rs::gc_stale_empty_sessions_deferred_until_settled` — Session：WP-166, wp166
- `src/client_protocol/ws_wp166_tests.rs::no_projection_frames_stranded_after_fastlane_list` — Session：wp166
- `src/client_protocol/ws_wp166_tests.rs::serve_state_does_not_run_snapshot_maintenance_inline` — Session：WP-166, wp166
- `src/client_protocol/ws_wp166_tests.rs::session_list_error_emits_operation_error_and_rpc_error` — Session：WP-166, wp166
- `src/client_protocol/ws_wp166_tests.rs::session_list_fastlane_matches_controller_list` — Session：WP-091, wp166
- `src/client_protocol/ws_wp166_tests.rs::session_list_not_blocked_by_inflight_subscribe` — Session：WP-091, wp166
- `src/client_protocol/ws_wp166_tests.rs::startup_settled_fires_on_first_new_reply_only` — Session：WP-166, wp166
- `src/client_protocol/ws_wp166_tests.rs::startup_settled_fires_without_list_or_new` — Session：WP-166, wp166
- `src/client_protocol/ws_wp166_tests.rs::subscribe_replay_frames_unchanged_with_concurrent_list` — Session：wp166
- `src/client_protocol/ws_wp166_tests.rs::subscribe_unsubscribe_order_preserved_with_list` — Session：wp166
- `src/config/global_db/seed.rs::migration_fills_missing_primary_code_search_only` — Session：wp149
- `src/config/global_db/seed.rs::migration_keeps_existing_false_row` — Session：wp149
- `src/config/global_db/seed.rs::migration_runs_once_on_repeated_startup` — Session：wp149
- `src/config/global_db/seed.rs::seed_plants_default_orchestrator_explore_and_general` — Session：WP-197
- `src/config/settings_writer.rs::commit_mixed_restores_file_when_db_step_fails` — Session：ISSUE-017, WP-008
- `src/config/settings_writer.rs::commit_mixed_restores_file_when_validate_fails` — Session：WP-149
- `src/config/settings_writer.rs::workspace_custom_without_rules_hides_global_rules` — Session：ISSUE-017, WP-008
- `src/config/settings_writer.rs::write_agent_existing_agent_switched_to_primary_not_auto_bound` — Session：wp149
- `src/config/settings_writer.rs::write_agent_new_primary_explicit_false_stays_false` — Session：wp149
- `src/config/settings_writer.rs::write_agent_new_primary_missing_code_search_binds_true` — Session：wp149
- `src/config/workspace.rs::load_workspace_state_deferred_does_not_warm` — Session：WP-166
- `src/config/workspace.rs::load_workspace_state_uses_external_snapshots_dir` — Session：WP-166
- `src/session/manager.rs::begin_turn_rejects_second_while_running` — Session：WP-024
- `src/session/manager.rs::cancel_during_starting_turn_cancels_on_start` — Session：wp090
- `src/session/manager.rs::cancel_leaves_pending_queue_intact` — Session：wp090
- `src/session/manager.rs::cancel_turn_sync_reports_idle_when_nothing_running` — Session：wp090
- `src/session/manager.rs::g3_turn_finished_reason_non_empty_on_error_termination` — Session：WP-024
- `src/session/manager.rs::g4_g5_compact_rejected_while_running_and_turn_restarts_after_error` — Session：WP-024
- `src/session/manager.rs::s1_reopen_prune_keeps_revision_in_sync` — Session：WP-024
- `src/session/manager.rs::turn_status_hook_fires_on_start_and_release` — Session：wp090
- `src/session/snapshot.rs::maintain_reads_active_ids_under_lock_no_false_orphan` — Session：WP-166
- `src/session/snapshot.rs::maintain_snapshots_waits_for_workspace_lock` — Session：WP-166
- `src/session/snapshot.rs::snapshot_track_skips_nested_git_dir_entry` — Session：WP-166
- `src/session/snapshot.rs::track_before_warm_matches_warm_then_track` — Session：WP-166
- `src/types/death_list_gate.rs::session_seq_g1_envelope_vocab_matches_mental_model` — Session：ISSUE-013
- `src/types/death_list_gate.rs::session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept` — Session：ISSUE-013
- `tests/client_protocol_project.rs::turn_finished_auto_compact_failure_is_compaction_failed` — Session：WP-024, WP-025, WP-069
- `tests/client_protocol_project.rs::turn_finished_marks_a_failed_reconnect_retryable` — Session：WP-024, WP-025, WP-069
- `tests/issue_032_changed_files_restore.rs::t1_spine_failure_restores_changed_files_for_next_seam` — Session：issue_032
- `tests/issue_032_changed_files_restore.rs::t2_files_changed_append_failure_soft_restores` — Session：issue_032
- `tests/issue_032_changed_files_restore.rs::t3_successful_files_changed_does_not_restore` — Session：issue_032
- `tests/issue_032_changed_files_restore.rs::t4_empty_changed_skips_restore` — Session：issue_032
- `tests/settings_api.rs::settings_retrieval_refresh_endpoint_accepts_when_cold` — Session：WP-131
- `src/client_protocol/connection.rs::retain_pending_on_disconnect_does_not_deny` — Session：WP-186（初稿未标）
- `src/client_protocol/connection.rs::abort_all_pending_is_aborted_not_deny` — Session：WP-186（初稿未标）
- `src/client_protocol/project.rs::snapshot_always_serializes_pending_permissions_even_when_empty` — Session：WP-186（初稿未标）
- `src/client_protocol/project.rs::wire_phase_writes_waiting_for_user_reads_alias` — Session：WP-186（初稿未标）
- `src/engines/session_search/mod.rs::agent_view_spills_the_remainder_into_a_named_file` — 上下文：**WP-205**：footer 文案改了以后，溢出文件名的解析要跟上（`named_spill_location`）。
- `src/tools/session_search.rs::search_hit_line_matches_virtual_read` — 上下文：**WP-205**：hit 行号要和虚拟 transcript 的 read（start_line+limit，data.lines）对齐。
- `src/tools/session_search.rs::a_result_over_budget_names_the_file_that_holds_the_rest` — 上下文：WP-205 时一并改动（共用 helper），属于协同回归，不在那两条之内。
- `src/context_pipeline/compact.rs::compact_persist_ok_reload_err_keeps_db_truth` — 上下文：ISSUE-035 / WP-039：持久化之后 reload 失败，必须以 DB 为准。
- `src/engines/session_search/semantic_index.rs::wp135_incremental_scans_then_embeds_from_kept_chunks` — 上下文：WP-135：增量 embed 和索引失败状态。
- `src/engines/session_search/semantic_index.rs::wp135_index_error_is_failed_index_failed_and_clears_on_success` — 上下文：WP-135：增量 embed 和索引失败状态。
- `src/engines/session_search/semantic_index.rs::wp144_bytes_per_batch_independent_of_index_size` — 上下文：WP-144：写放大。
- `src/engines/session_search/mod.rs::gate_drops_weak_semantic` — 上下文：WP-057：宁可不返回，也不要假命中。
- `src/engines/session_search/mod.rs::gate_keeps_strong_semantic` — 上下文：WP-057：宁可不返回，也不要假命中。
- `src/engines/code_search/reconcile.rs::crash_between_index_commit_and_stamps_rereads_never_misses` — 上下文：WP-143 / WP-141。
- `tests/code_search_ipc.rs::refresh_stays_incremental_when_all_files_dirty` — 上下文：ISSUE-052 / WP-057。
- `tests/manual_compact_e2e.rs::manual_compact_failure_e2e_releases_lease_and_retry_starts` — 上下文：WP-025。
- `tests/manual_compact_e2e.rs::manual_compact_retry_after_failure_completes` — 上下文：WP-025。
- `tests/optional_code_search.rs::catalog_lists_tool_while_warming_and_after_warm` — 上下文：WP-148
- `tests/optional_code_search.rs::code_index_tick_syncs_without_refresh` — 上下文：WP-079
- `tests/optional_code_search.rs::code_search_lifecycle_rebuild_stale_consume_stop` — 上下文：WP-143
- `tests/optional_code_search.rs::pipeline_version_change_triggers_rebuild` — 上下文：WP-143
- `tests/optional_code_search.rs::warmup_loads_compatible_index_without_full_rebuild` — 上下文：WP-143
- `tests/stage_a_ctx_consistency.rs::manual_compact_bypasses_auto_threshold_and_preserves_full_history` — 上下文：ISSUE-013
- `tests/stage_a_ctx_consistency.rs::pipeline_compact_reloads_working_set_from_fold` — 上下文：ISSUE-013
- `src/knowledge/spec_tests.rs::directory_texts_match_the_status` — 上下文：WP-050
- `src/knowledge/spec_tests.rs::lines_on_a_symbol_are_invalid_lines_in_the_snapshot` — 上下文：WP-050
- `src/knowledge/spec_tests.rs::oversized_and_binary_files_are_unreadable_everywhere` — 上下文：WP-050
- `src/llm/codec/chat.rs::recorded_reasoning_is_replayed_as_reasoning_content` — Provider：Comment: regression guard — model-owned reasoning must replay as `reasoning_content` regardless of item id.
- `src/llm/codec/stream_contract.rs::duplicate_item_id_in_completed_output_is_refused` — Provider：Comment: duplicate terminal ids caused two transcript rows; must error.
- `src/workspace/tool_path.rs::symlinked_dir_escape_rejected_for_new_file` — IDE：修复 1.5，词法 fallback 能绕过 is_under，导致新文件写到工作区外
- `src/workspace/service.rs::refuses_symlink_copy_escaping_workspace` — IDE：复制 symlink 时外泄内容，或留下半成品链接
- `tests/workspace_path_lap.rs::boot_sandbox_lsp_os_probe_roots_are_identical_lap` — IDE：各处的根路径形态不一致
- `src/lsp/mod.rs::publish_diagnostics_uri_matches_drive_case` — IDE：盘符大小写或 verbatim 形态导致诊断丢失
- `src/lsp/mod.rs::uri_to_path_recovers_mangled_verbatim_file_uri` — IDE：盘符大小写或 verbatim 形态导致诊断丢失
- `src/lsp/conn.rs::unversioned_publish_after_change_is_silence_not_stale_errors` — IDE：修好代码后仍回显上一轮的错误
- `src/lsp/conn.rs::edit_fix_must_not_echo_previous_round_error` — IDE：修好代码后仍回显上一轮的错误
- `tests/optional_lsp.rs::edit_fix_must_not_echo_previous_lsp_error` — IDE：修好代码后仍回显上一轮的错误
- `tests/optional_lsp.rs::get_diagnostics_rpc_does_not_deadlock` — IDE：死锁
- `tests/optional_lsp.rs::sync_then_feedback_on_current_thread_does_not_deadlock` — IDE：死锁
- `tests/optional_lsp.rs::idle_reclaim_during_diagnostics_does_not_panic` — IDE：回收过程中 panic
- `tests/optional_lsp.rs::request_timeout_does_not_kill_process` — IDE：超时误杀 LS 进程
- `tests/optional_lsp.rs::overlapping_hovers_match_out_of_order_ids` — IDE：响应乱序后错配
- `src/lsp/conn.rs::dispatch_out_of_order_responses` — IDE：响应乱序后错配
- `src/lsp/sweep_gate.rs::gate_opens_when_sweep_panics` — IDE：sweep 误杀刚 spawn 的 LS，或 sweep panic 后永久阻塞（WP-165）
- `src/lsp/mod.rs::spawn_ls_waits_for_sweep_gate` — IDE：sweep 误杀刚 spawn 的 LS，或 sweep panic 后永久阻塞（WP-165）
- `src/optional/mod.rs::defer_reconcile_builds_no_client` — IDE：启动时提前预热；锁中毒引起连锁 panic
- `src/optional/mod.rs::poisoned_state_lock_does_not_cascade_panics` — IDE：启动时提前预热；锁中毒引起连锁 panic
- `src/lsp/project_root.rs::rust_analyzer_uses_cargo_workspace_not_member_crate` — IDE：同一 workspace 里多开 RA，或嵌套 crate 选错根
- `src/lsp/project_root.rs::rust_nested_crate_outside_workspace_members_keeps_own_root` — IDE：同一 workspace 里多开 RA，或嵌套 crate 选错根
- `src/workspace/git.rs::inherited_git_dir_env_does_not_select_snapshot_repo` — IDE：继承的 GIT_DIR 指向快照仓
- `src/workspace/git.rs::git_commands_skip_optional_index_locks` — IDE：index.lock 冲突
- `src/workspace/watcher.rs::atomic_save_overwrite_is_reclassified_as_modified` — IDE：原子保存被误报为删除
- `src/workspace/watcher.rs::delete_then_recreate_coalesces_to_modified` — IDE：原子保存被误报为删除
- `src/workspace/service.rs::change_broadcast_is_bounded_and_lagged_receiver_recovers` — IDE：广播无上界，lag 后无法恢复
- `src/workspace/filter/index_policy.rs::search_exclude_does_not_gate_code_index_queue` — IDE：3ddc643，search_exclude 误拦了代码索引
- `src/lsp/install/npm.rs::npm_is_invoked_as_the_shim_not_via_cmd_exe` — IDE：安装和探测相关的回归
- `src/lsp/install/adapters/typescript.rs::pins_typescript_below_7` — IDE：安装和探测相关的回归
- `src/lsp/install/github_tests.rs::asset_exact_name_skips_vsix` — IDE：安装和探测相关的回归
- `src/lsp/deps.rs::rustup_link_without_component_is_unavailable` — IDE：安装和探测相关的回归
- `src/lsp/deps.rs::rust_analyzer_symlink_to_rustup_spawns_as_rust_analyzer` — IDE：安装和探测相关的回归
- `tests/workspace_process_contract.rs::workspace_open_http_endpoint_is_gone` — IDE：已移除的接口不能复活
- `src/engines/code_search/index_status.rs::decide_index_work_dirty_always_updates` — 上下文：按组登记展开（WP-057/WP-131 断言）
- `src/engines/code_search/index_status.rs::decide_index_work_deletions_stay_on_update` — 上下文：按组登记展开（WP-057/WP-131 断言）
- `src/engines/code_search/index_status.rs::removed_legacy_helpers_stay_gone` — 上下文：按组登记展开（WP-057/WP-131 断言）
- `tests/workspace_path_death_list.rs::death_list_no_bare_canonicalize_outside_path_module` — IDE：按组登记展开（裸 canonicalize/verbatim）
- `tests/workspace_path_death_list.rs::death_list_web_no_silent_verbatim_strip` — IDE：按组登记展开（裸 canonicalize/verbatim）
- `tests/workspace_path_death_list.rs::death_list_src_canonicalize_whitelist_is_only_path_rs` — IDE：按组登记展开（裸 canonicalize/verbatim）

按组登记的回归已展开到上表：上下文 index_status（含 WP-057/WP-131 断言的 3 条）、IDE workspace_path_death_list ×3；Provider 那一组只是产品代码注释提到 WP-188，没有对应测试，不计。

## 3. 守护缺口（17 条，没有真正守护的行为）

> 建议另开补测包处理，不放在 WP-208。

1. [IDE] 已存在的 symlink 指向工作区外，经 Safe/人工 API 直接读、写、编辑无测试（只有新文件 L2、复制 L5 两种情形）
2. [IDE] 绝对路径内含 `..`（`<ws>/../x`，Safe 且目标不存在）无测试
3. [IDE] project_root 向上找祖先时越过工作区：停止条件是原始相等比较，verbatim/盘符大小写不同可能选中工作区外 marker，无测试
4. [IDE] Sandbox::rel_path / RelPathCtx::rel 用 starts_with 而非 is_under，Windows 盘符大小写无测试
5. [IDE] 敏感路径（.env/.ssh）实现在 permission::sensitive，本域无守护，需 permission 侧确认
6. [IDE] Windows 链路仅 [win] 测试（L3、L7 部分、L2 windows 版），Linux CI 不跑，symlink 版无权限时静默跳过
7. [IDE] lsp/uri.rs、server_map.rs、paths.rs 无自身单测，paths.rs 无覆盖（另：真实 RA 冒烟依赖本机环境，CI 可能从未真正执行）
8. [Runtime] src/proc_lifetime.rs 0 单测：bind_child_to_parent / Windows job kill-on-close / sweep_once 三平台实现无行为测试，仅静态死名单间接覆盖
9. [Tools] `run.rs::wait_yields_so_cancel_can_run` 零断言（`let _ = (result, jobs)`），不守任何行为
10. [Tools] webfetch DNS 钉扎（ISSUE-021 rebinding）只有 `client_pins_hostname_*` 冒烟，不验证不二次解析
11. [Tools] write 原子性：`write.rs::test_write_atomic` 只验读回；原子性实际只由 wp028 两条竞态探针间接守护
12. [Session] set-model 与 in-flight turn：运行中切模型（拒绝/排队/下一 turn 生效）无测试，也无 RPC 级端到端
13. [Session] CLI 入口：参数解析 / serve 子命令启动无守护（仅 4 个答题 helper）
14. [Session] authority.rs 权威来源判定语义无守护（仅 2 个 roundtrip）
15. [Session] version.rs 仅 channel 非空 / dev 冒烟
16. [Session] telemetry 日志订阅/上报无行为测试（client_protocol_telemetry 仅 serde）
17. [Session] WS 握手+鉴权+subscribe 冷启动 hydrate 一体化：无「鉴权后 subscribe → snapshot 含 pending_permissions」端到端

## 4. GM 决议（2026-10-22）

1. Session C15 snapshot 回滚（issue_032 那 4 条）算核心，归 Session，已加进守护（见 session/snapshot）。
2. serve 绑定鉴权：保留 `tests/serve_bind_auth.rs::bind_auth_matrix` 与 `src/serve/run.rs::loopback_only_rejects_non_loopback_even_with_auth`；删除 `src/serve/run.rs` 中重复的 `case_a_loopback_require_auth_with_token_ok`、`case_b_non_loopback_require_auth_with_token_ok`、`case_c_non_loopback_without_auth_fails`、`case_d_require_auth_without_token_fails`。
3. `src/config/path.rs` 4 条归 IDE（L7）。
4. 不在任何域范围内的测试不动；17 条缺口本包不补。

## 附：核对中引用但 3acfe31 上不存在的测试名（未计入守护）

- [Provider] `replay_is_quirk_gated_and_synthesizes_missing_reasoning`
- [Provider] `summary_replay_requires_responses_and_rejects_the_text_quirk`

## 5. 汇总

| 域 | 范围内总数 | 核心守护（去重后归属） | 非核心 |
|---|---:|---:|---:|
| Tools | 616 | 173 | 443 |
| Runtime | 278 | 121 | 157 |
| Session | 585 | 99 | 486 |
| 上下文 | 568 | 81 | 487 |
| Provider | 221 | 72 | 149 |
| IDE | 364 | 119 | 245 |
| **合计** | **2632** | **665** | **1967** |

- 链路：108 条；守护测试（去重）：665；回归测试（逐条，去重）：183（含按组登记展开的 6 条），其中 84 条同时是守护；守护缺口：17。
- 范围合计 2632 与 3acfe31 全量（`--list` 2714 / 源码计 2669）的差：各域范围边界不完全覆盖（如 tests/ 中 architecture_guards 以外的少量文件、types/agent 等）及 Session 范围与 Tools 范围的少量交叠，属估算口径差异。
- 回归测试不在守护表内的约 99 条：删减前需逐条确认是否保留（默认保留）。

非核心按类别估算（各域 §4 合并，约数）：

| 类别 | 约数 | 来源 |
|---|---:|---|
| 纯函数/边角/内部实现单元（文件工具细节、session_search/code_search 纯函数、planner/matcher 等） | 300 | Tools 102、上下文 190、IDE 其余 |
| 格式/序列化/文案/wire 形状 | 200 | 上下文 110、Session 45、Runtime observer/phase、Provider encode 矩阵部分 |
| 配置面 CRUD/校验边角、预设/目录清单 | 150 | Session 114、Tools 权限预设 19、Provider catalog 边角 28 |
| 一致性/重复覆盖（parity/golden、同一判定的变体、冗余集成） | 200 | 上下文 80、Session sqlite 变体 90、Runtime 集成 45、Tools 集成 |
| provider codec 编码/折叠/synth 边角 | 110 | Provider |
| 工具产品面（subagent/todo/plan/ask_user/knowledge/workspace 工具）与命令/网络细节 | 145 | Tools 91+31+21 |
| IDE 功能（filter/watcher/text_codec/tree/install/deps/git 等） | 200 | IDE |
| 架构/源码扫描守卫（非核心部分）、冒烟、性能/规模、状态/进度 | 100 | Session 18+6、上下文 45+6、Runtime/IDE 其余 |

（类别为粗估，合计与上表非核心数不严格相等。）
