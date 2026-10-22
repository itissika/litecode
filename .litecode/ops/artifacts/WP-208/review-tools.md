# WP-208 核对 · Tools 域（基准 3acfe31，只读人工核对）

**摘要**：核心链路 **18** 条；守护测试 **173** 条（逐条读体确认断言）；对初稿：补 **61**、剔 **95**；回归测 **32** 条；范围内共 616 条测试，其余 **429** 条为非核心。

**要点**：
- 初稿把范围内 tests/ 67 条整体判为守护（未筛）；且 edit 链名仍写 `replace_all`（已在 WP-198 退役）；read/glob 链名声称 EOF 空 lines / basename 深度匹配，却未列对应测试；MCP、命令混淆绕过（bash_safety）、schema 校验三块整体缺失。
- **无真守护**：`run.rs::wait_yields_so_cancel_can_run` 零断言；webfetch DNS 钉扎（ISSUE-021 rebinding）只有 `client_pins_hostname_*` 冒烟（只建 client，不验证不二次解析）；`write.rs::test_write_atomic` 只验读回、不验原子性——原子性实际只由 wp028 两条竞态探针守护。
- `tests/f2_lock_scope.rs` 属 session/manager 域，不在 Tools 链内。
- "merged from phaseN_gate, WP-207" 是合并来源注释，不算回归引用（初稿据此误列 grep/read/edit/write/executor 若干条）。

## 1. 核心链路 → 守护测试

### C1（8）
守：read：1-based start_line/limit 分页，默认页有界并提示下一页；越 EOF（含空文件）resolve 空 lines+message「共 N 行」；缺文件 ToolError not_found；长行在 text 内截断；工作区外路径可读并标注；取消在分块间生效。

- `src/tools/read.rs::test_read_with_start_line_and_limit`
- `src/tools/read.rs::test_read_default_limit_pages`
- `src/tools/read.rs::test_read_start_line_past_end_resolves_empty`
- `src/tools/read.rs::test_read_empty_file_past_end_resolves_empty`
- `src/tools/read.rs::read_missing_file_is_not_found`
- `src/tools/read.rs::test_read_long_line_is_capped_in_text`
- `src/tools/read.rs::outside_workspace_paths_noted`
- `src/tools/read.rs::cancelled_read_stops_between_chunks`

### C2（7）
守：grep：结果为有界 hits（limit 触顶进 message、不落 spill 文件）；非法 regex / 空 glob 为 invalid_input；排除树/二进制/空 glob 等结果态走 resolve+message。

- `src/tools/grep.rs::limit_bounds_hits_and_notes_truncation`
- `src/tools/grep.rs::wide_search_never_writes_a_spill_file`
- `src/tools/grep.rs::a_bare_brace_is_invalid_regex`
- `src/tools/grep.rs::blank_glob_is_invalid_input`
- `src/tools/grep.rs::grep_message_path_excluded`
- `src/tools/grep.rs::grep_message_binary_file`
- `src/tools/grep.rs::grep_message_glob_empty`

### C3（5）
守：glob：无 / 的模式按文件名任意深度匹配；含 .. 的模式 invalid_input；默认尊重 gitignore；触顶截断进 message；取消在遍历前生效。

- `src/tools/glob.rs::basename_glob_recurses`
- `src/tools/glob.rs::parent_dir_pattern_is_invalid_input`
- `src/tools/glob.rs::default_filters_hide_gitignored`
- `src/tools/glob.rs::truncates_with_message`
- `src/tools/glob.rs::cancelled_glob_stops_before_walking`

### C4（6）
守：write：expected_hash 不符 → stale 且不落盘、缺文件带 hash 不创建；create_only 遇已存在 → conflict 不覆盖；虚拟 session 路径只读；敏感路径在 run_code 内同样被 floor 拒绝。

- `src/tools/write.rs::write_expected_hash_mismatch_is_stale`
- `src/tools/write.rs::write_expected_hash_on_missing_file_does_not_create`
- `src/tools/write.rs::write_expected_hash_match_returns_file_ref`
- `src/tools/write.rs::test_write_create_only_refuses_overwrite`
- `src/tools/write.rs::virtual_session_path_is_read_only`
- `tests/code_surface.rs::nested_write_sensitive_path_is_denied_by_the_same_floor`

### C5（12）
守：edit/apply_edits（WP-198 契约）：扁平 old/new，无 replace_all/expected_hash；唯一匹配替换、空 new 删除；未命中/多匹配 resolve ok:false+message（报行号、不提 replace_all）且不落盘；apply_edits 同 path 真串行、单条失败不中断且 results 等长；缩进仅差异按结构拒绝。

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

### C6（11）
守：run/wait/kill：短命令前台返回 exit_code；超窗返回 exit_code=null 且进程续跑（promote）；资源锁只持有到 spawn（并行写不串行化，spawn 失败/future 丢弃/panic 都释放）；前台窗受 run_code deadline 封顶；wait 超时保持 null、退出后带码；kill 真杀进程；未知 id not_found。

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

### C7（11）
守：LSP 七工具（WP-202）：七个 snake_case 名且仅 Code 面；定位入参 file_path+1-based line/UTF-16 character，越界/目录/根外 ToolError；workspace_symbol 仅 query 且每条带相对 path；空结果 resolve+[]+message；未暖 unavailable（非空成功）。

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

### C8（9）
守：webfetch/websearch（WP-203）：SSRF 拒 loopback/私网/100.64/IPv4-mapped，放行 198.18/15；非 http/私网/未 warm 为硬 ToolError（invalid_input/permission_denied/unavailable）；HTTP/Exa 失败软 resolve ok:false 带 url/status 或 query。

- `src/tools/webfetch.rs::ssrf_rejects_loopback_and_private_ips`
- `src/tools/webfetch.rs::ssrf_shared_cgnat_still_non_public`
- `src/tools/webfetch.rs::ssrf_allows_benchmarking_fake_ip_range`
- `src/tools/webfetch.rs::private_ip_is_hard_permission_denied`
- `src/tools/webfetch.rs::non_http_scheme_is_hard_invalid_input`
- `src/tools/webfetch.rs::not_warmed_is_hard_unavailable`
- `src/tools/webfetch.rs::soft_fail_shape_has_ok_false_without_error_level`
- `src/tools/websearch.rs::soft_fail_shape_resolves_ok_false`
- `src/tools/websearch.rs::not_warmed_surfaces_configuration_hint`

### C9（11）
守：run_code 结果面（WP-200）：只渲染 head+logs+result，在预算内保尾并标注省略；成功/失败只计已结算调用，未结算单独旁注；by-tool data 不泄入 content；第二个 JS 参数在工具运行前拒绝；沙箱无 process/fs/module。

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

### C10（9）
守：Code 面投影与路由：Code 工具直调被拒（授权前）并经 run_code 成功；不可见工具 not_found 且不执行；嵌套调用保留工具自身权限、deny 不执行；无 data 契约/不可渲染工具不进两面；子深度按调用者可见集投影。

- `tests/code_surface.rs::direct_code_tools_are_refused_and_succeed_through_run_code`
- `tests/code_surface.rs::direct_edit_and_write_are_refused_before_authorization`
- `tests/code_surface.rs::code_call_of_invisible_tool_is_not_found_and_does_not_run`
- `tests/code_surface.rs::code_call_keeps_the_tools_own_permission_and_does_not_execute_a_deny`
- `tests/code_surface.rs::code_success_without_matching_data_is_an_internal_error`
- `tests/code_surface.rs::missing_or_unrenderable_code_tool_leaves_both_surfaces`
- `src/tool/registry.rs::code_gate_projects_every_depth_from_that_callers_visible_set`
- `src/tool/registry.rs::webfetch_websearch_project_to_code_not_direct`
- `src/tool/code_manifest.rs::rejects_tools_without_a_data_contract`

### C11（8）
守：嵌套调用取消/超时/grace 封口：取消封住在途嵌套调用与写；超 grace 记为 unsettled 且迟到的批准/拒绝不写盘；外层超时不覆盖已完成结果；等待审批不扣程序预算。

- `tests/code_surface.rs::cancellation_seals_in_flight_nested_calls`
- `tests/code_surface.rs::cancelled_run_code_seals_an_in_flight_write_and_edit`
- `tests/code_surface.rs::a_call_that_outlasts_the_grace_period_is_unsettled`
- `tests/code_surface.rs::grace_expiry_is_unsettled_and_the_later_denial_does_not_write`
- `tests/code_surface.rs::cancel_before_approval_does_not_write_later`
- `tests/code_surface.rs::outer_timeout_does_not_overwrite_a_completed_nested_result`
- `tests/code_surface.rs::approval_wait_does_not_consume_the_program_budget`
- `tests/code_surface.rs::nested_write_approval_denial_does_not_apply_the_edit`

### C12（8）
守：执行器与入参校验：已提交结果优先于超时/迟到取消；阻塞工具被管线超时打断；Code 调用未知参数 invalid_input，非法 JSON 不被吞；schema 校验必填/嵌套未知字段/oneOf。

- `src/tool/executor.rs::committed_result_wins_over_tool_timeout`
- `src/tool/executor.rs::completed_tool_result_wins_over_late_cancellation`
- `src/tool/executor.rs::blocking_tool_is_interrupted_by_pipeline_timeout`
- `src/tool/executor.rs::unknown_parameter_is_invalid_input`
- `src/tool/executor.rs::invalid_json_arguments_are_not_swallowed`
- `src/tool/schema_validate.rs::missing_required`
- `src/tool/schema_validate.rs::additional_properties_false_rejects_nested_unknown_field`
- `src/tool/schema_validate.rs::one_of_discriminated_union`

### C13（15）
守：资源锁：读共享/写独占，等待中的写不被后来读插队，同 key 写按提交序，多 key 排序防死锁，排队可取消，key 规范化；同路径不同拼写只有一个 create_only 赢家且读等写；审批挂起不预占也不阻塞他人；跨会话写锁无部分获取。

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

### C14（28）
守：权限底线：floor（危险命令/敏感写）在 all 预设与 always 授权下仍硬拒且不问；safe 只读命令放行其余 deny；子代理 ask→deny；首条匹配规则胜出；always 授权按 rule+agent 限定；sink 取消为 aborted 非 deny；保留名/run_code 同名不得绕过（按 transport 身份）；命令混淆绕过被拒。

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

### C15（12）
守：MCP：每连接仅一次 initialize；JSON-RPC 读跳过通知/他 id，超时/取消标 dirty 并拒复用、重建后可用；崩溃后池重建、同 key 复用；kill/drop 回收孙进程；hub 挂起时 execute 保持 pending；启动失败不广告假工具。

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

### C16（3）
守：契约快照：run_code SDK 文本+sha 与 fixture 逐字一致；各工具 data/error 形状锁定在 baseline。

- `src/tool/sdk_snapshot.rs::phase0_run_code_sdk_snapshot`
- `src/tool/sdk_snapshot.rs::sdk_hash_is_stable_for_the_same_tool_set_under_different_cwd_and_permission`
- `src/tools/baseline/mod.rs::phase0_tool_fixtures_lock_data_and_error`

### C17（3）
守：底层文件原子性（write/create_only 依赖）：并发 create/不覆盖写至多一个成功；跨设备覆盖移动成功。

- `tests/wp028_probe_s2_create_race.rs::probe_s2_create_file_race`
- `tests/wp028_probe_s2_create_race.rs::probe_s2_write_file_bytes_no_overwrite_race`
- `tests/wp028_probe_s1_xdev_move.rs::probe_s1_cross_device_overwrite_succeeds`

### C18（7）
守：检索类工具（code_search/session_search）：引擎非 Warm/失败时立即返回、不发起搜索、首行固定 sync 状态；Warm+Stale 仍可搜；session_search 超预算点名溢出文件、命中行号与虚拟 read 一致。

- `src/tools/code_search.rs::warming_returns_immediately_without_sleeping`
- `src/tools/code_search.rs::warm_failed_index_job_returns_error_not_hits`
- `src/tools/code_search.rs::not_ready_issues_no_search`
- `src/tools/code_search.rs::warm_stale_is_searchable_without_warm_wait`
- `src/tools/session_search.rs::semantic_unavailable_is_explicit_and_leads_with_sync_pct`
- `src/tools/session_search.rs::a_result_over_budget_names_the_file_that_holds_the_rest`
- `src/tools/session_search.rs::search_hit_line_matches_virtual_read`

## 2. 对初稿的修正

### 补（61）

- `src/mcp/client.rs::drop_reaps_grandchild_process` — MCP 读循环/孙进程回收 R1–R3 与 kill/drop，初稿整条 MCP 链缺失
- `src/mcp/client.rs::kill_reaps_grandchild_process` — MCP 读循环/孙进程回收 R1–R3 与 kill/drop，初稿整条 MCP 链缺失
- `src/mcp/client.rs::r1_skips_notification_before_response` — MCP 读循环/孙进程回收 R1–R3 与 kill/drop，初稿整条 MCP 链缺失
- `src/mcp/client.rs::r2_hang_times_out_and_marks_dirty` — MCP 读循环/孙进程回收 R1–R3 与 kill/drop，初稿整条 MCP 链缺失
- `src/mcp/client.rs::r3_cancel_marks_dirty_next_client_ok` — MCP 读循环/孙进程回收 R1–R3 与 kill/drop，初稿整条 MCP 链缺失
- `src/tool/code_manifest.rs::rejects_tools_without_a_data_contract` — 无 data 契约工具不得进 Code 面（准入）
- `src/tool/executor.rs::invalid_json_arguments_are_not_swallowed` — 非法 JSON 不被吞成空入参
- `src/tool/executor.rs::partition_splits_same_path_read_and_write` — 同路径读写必须拆批，资源锁正向守护
- `src/tool/registry.rs::code_gate_projects_every_depth_from_that_callers_visible_set` — 子深度按调用者可见集投影
- `src/tool/registry.rs::custom_tools_cannot_take_command_or_retired_shell_names` — 自定义工具不得占用命令/旧 shell 保留名（权限绕过面）
- `src/tool/registry.rs::failed_mcp_start_does_not_advertise_dummy_tool` — MCP 启动失败不广告假工具
- `src/tool/registry.rs::mcp_inner_names_cannot_use_reserved_command_names` — MCP 内名不得用保留命令名
- `src/tool/registry.rs::webfetch_websearch_project_to_code_not_direct` — 网络工具仅 Code 面（WP-203 终态）
- `src/tool/schema_validate.rs::additional_properties_false_rejects_nested_unknown_field` — 入参 schema 校验是 invalid_input 的源头
- `src/tool/schema_validate.rs::missing_required` — 入参 schema 校验是 invalid_input 的源头
- `src/tool/schema_validate.rs::one_of_discriminated_union` — 入参 schema 校验是 invalid_input 的源头
- `src/tools/apply_edits.rs::schema_rejects_retired_knobs` — apply_edits 拒退役旋钮
- `src/tools/apply_edits.rs::serial_skip_continues_results_same_len` — WP-198 B：同 path 真串行、失败不中断、results 等长
- `src/tools/bash_safety.rs::bypass_backslash_escaped_rm_is_denied` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/bash_safety.rs::bypass_backtick_rm_is_denied` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/bash_safety.rs::bypass_command_substitution_rm_is_denied` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/bash_safety.rs::bypass_eval_rm_is_denied` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/bash_safety.rs::bypass_nested_obfuscation_is_denied` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/bash_safety.rs::bypass_quote_wrapped_rm_is_denied` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/bash_safety.rs::bypass_readonly_classification_is_not_foiled` — 命令混淆（引号/反斜杠/$( )/`/eval）绕过 floor 的安全守护，初稿漏
- `src/tools/code.rs::unfinished_call_stays_unsettled_even_when_the_host_later_records_ok` — WP-200 A/B：未结算不计成功/失败
- `src/tools/code/render.rs::failure_header_counts_and_marks_unsettled` — 失败头计数+unsettled 旁注
- `src/tools/code/render.rs::large_result_keeps_the_tail_and_states_the_omission` — 结果超预算保尾并标注省略
- `src/tools/code/render.rs::success_header_counts_failures_from_call_states` — 成功头按调用状态计失败数
- `src/tools/code_search.rs::not_ready_issues_no_search` — 非 Warm 不阻塞/不搜/首行 sync（C18 新链）
- `src/tools/code_search.rs::warm_failed_index_job_returns_error_not_hits` — 非 Warm 不阻塞/不搜/首行 sync（C18 新链）
- `src/tools/code_search.rs::warming_returns_immediately_without_sleeping` — 非 Warm 不阻塞/不搜/首行 sync（C18 新链）
- `src/tools/edit/tests.rs::edit_failure_resolves_with_message` — WP-198 扁平契约/ok:false 软失败/多匹配报行不落盘的直接守护（初稿用 planner 内部函数替代）
- `src/tools/edit/tests.rs::edit_tool_schema_accepts_flat_shape_rejects_old` — WP-198 扁平契约/ok:false 软失败/多匹配报行不落盘的直接守护（初稿用 planner 内部函数替代）
- `src/tools/edit/tests.rs::edit_tool_wire_multiple_matches` — WP-198 扁平契约/ok:false 软失败/多匹配报行不落盘的直接守护（初稿用 planner 内部函数替代）
- `src/tools/edit/tests.rs::empty_new_string_deletes_old` — WP-198 扁平契约/ok:false 软失败/多匹配报行不落盘的直接守护（初稿用 planner 内部函数替代）
- `src/tools/edit/tests.rs::single_success_reports_edit_result` — WP-198 扁平契约/ok:false 软失败/多匹配报行不落盘的直接守护（初稿用 planner 内部函数替代）
- `src/tools/edit/tests.rs::unmatched_reports_no_match_without_applying` — WP-198 扁平契约/ok:false 软失败/多匹配报行不落盘的直接守护（初稿用 planner 内部函数替代）
- `src/tools/glob.rs::basename_glob_recurses` — 初稿链名声称 basename 任意深度却未列此测试
- `src/tools/glob.rs::default_filters_hide_gitignored` — 默认尊重 gitignore
- `src/tools/glob.rs::truncates_with_message` — 触顶截断进 message
- `src/tools/lsp.rs::character_out_of_range_is_hard_error` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/lsp.rs::character_utf16_multibyte` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/lsp.rs::code_tool_names_are_snake_case_seven` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/lsp.rs::finish_locations_empty_resolves_with_empty_array` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/lsp.rs::finish_symbols_workspace_paths_relative` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/lsp.rs::position_schema_requires_file_path_line_character` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/lsp.rs::workspace_symbol_schema_is_query_only` — WP-202 七名/入参/越界/空结果/workspace path 契约直接守护
- `src/tools/read.rs::test_read_default_limit_pages` — 默认页有界并提示下一页 start_line
- `src/tools/read.rs::test_read_empty_file_past_end_resolves_empty` — 初稿链名声称 EOF 空 lines 却未列（WP-206 ④）
- `src/tools/read.rs::test_read_long_line_is_capped_in_text` — 行截断落在 text 内（WP-199 #6）
- `src/tools/read.rs::test_read_start_line_past_end_resolves_empty` — 同上，WP-206 ④
- `src/tools/run.rs::long_command_returns_null_exit_and_keeps_job` — 超窗 exit_code=null 且进程续跑（promote）唯一正向守护
- `src/tools/session_search.rs::search_hit_line_matches_virtual_read` — 命中行号与虚拟 read 一致（C18）
- `src/tools/webfetch.rs::not_warmed_is_hard_unavailable` — 未 warm=硬 unavailable
- `src/tools/webfetch.rs::soft_fail_shape_has_ok_false_without_error_level` — 软失败 ok:false 带 url/status，非 Error 级
- `src/tools/webfetch.rs::ssrf_shared_cgnat_still_non_public` — 100.64/10 仍拒
- `src/tools/websearch.rs::not_warmed_surfaces_configuration_hint` — 未 warm 带配置提示
- `src/tools/websearch.rs::soft_fail_shape_resolves_ok_false` — Exa 失败软 ok:false
- `src/tools/write.rs::test_write_create_only_refuses_overwrite` — create_only 不覆盖（conflict）
- `src/tools/write.rs::virtual_session_path_is_read_only` — 虚拟 session 路径只读

### 剔（95）

- `src/permission/engine.rs::all_and_removed_rules_allow` — 预设细节，非底线
- `src/permission/engine.rs::safe_uses_the_live_rules` — 规则装载细节
- `src/permission/engine.rs::workspace_tool_without_rules_drops_the_global_rules` — 规则合并细节
- `src/permission/floor.rs::retired_direct_shell_name_does_not_hit_the_command_floor` — 退役名负例，非核心
- `src/permission/grants.rs::cancellable_wait_returns_none_when_cancelled` — 内部等待工具函数
- `src/permission/grants.rs::grant_has_upper_bound_per_agent` — 容量上限，非安全底线
- `src/permission/grants.rs::wp081_legacy_q0_free_text_always_true` — ask_user 自由文本，非权限底线（列回归）
- `src/permission/matchers.rs::arg_equals_matches` — 匹配器单元，first_matching_rule_wins 已覆盖
- `src/permission/matchers.rs::glob_star_matches_all` — 匹配器单元，first_matching_rule_wins 已覆盖
- `src/permission/messages.rs::other_tools_keep_generic_message` — 文案，非行为
- `src/permission/messages.rs::run_floor_deny_explains_safety_rule` — 文案，非行为
- `src/permission/messages.rs::run_safe_default_deny_explains_readonly_policy` — 文案，非行为
- `src/permission/messages.rs::user_denial_mentions_no_retry` — 文案，非行为
- `src/permission/presets.rs::coding_safe_rules_stay_in_place` — 预设清单形状
- `src/permission/presets.rs::litecode_workspace_safe_asks_only_on_refresh` — 预设清单形状
- `src/permission/presets.rs::preset_surface_is_opt_in` — 预设清单形状
- `src/permission/sensitive.rs::detects_unix_and_windows_system_paths` — 由 floor_sensitive_write 端到端覆盖
- `src/tool/executor.rs::cancellable_tools_opt_in_and_the_default_is_off` — 默认值声明
- `src/tool/executor.rs::cross_session_lock_follows_effect_writes` — 声明映射，write_lock 已守
- `src/tool/executor.rs::partition_keeps_different_path_reads_concurrent` — 并发优化，非正确性
- `src/tool/executor.rs::write_outside_workspace_reports_path_outside_workspace` — 错误文案码
- `src/tool/write_lock.rs::test_acquire_release` — 基本 acquire/重入/粗锁，冲突/无部分获取两条已足
- `src/tool/write_lock.rs::test_reentrant_same_session` — 基本 acquire/重入/粗锁，冲突/无部分获取两条已足
- `src/tool/write_lock.rs::test_workspace_coarse_lock` — 基本 acquire/重入/粗锁，冲突/无部分获取两条已足
- `src/tools/baseline/mod.rs::phase0_uncovered_scenarios_are_recorded_as_gaps` — 只锁"仍是缺口"的记账，非行为
- `src/tools/code/render.rs::a_log_that_fills_its_budget_keeps_a_short_result_verbatim` — 渲染细节
- `src/tools/code/render.rs::cjk_result_stays_valid_utf8_inside_the_budget` — 渲染细节（UTF-8 边界）
- `src/tools/code/render.rs::dropped_undefined_line_sits_in_the_head` — 渲染细节（列回归）
- `src/tools/code_search.rs::agent_view_tight_budget_drops_to_context_without_breadcrumb` — code_search 排版，误归 run_code 渲染链
- `src/tools/code_search.rs::agent_view_tiny_budget_truncates_hit_count` — code_search 排版，误归 run_code 渲染链
- `src/tools/code_search.rs::fence_escapes_backticks_in_source` — 排版细节
- `src/tools/code_search.rs::indexing_wait_message_reads_job_json` — 文案（列回归）
- `src/tools/code_search.rs::warm_busy_worker_returns_warming_without_blocking` — C18 已由 warming_returns_immediately 覆盖
- `src/tools/code_search.rs::warm_refresh_busy_does_not_block_on_wait` — 同上
- `src/tools/code_search.rs::warm_refreshing_does_not_block_on_wait` — 同上
- `src/tools/edit/matcher.rs::multiline_indent_only_is_structural` — 内部匹配器，C5 已留 structural_prefix
- `src/tools/edit/matcher.rs::multiple_match_message_lists_lines` — 内部函数，工具级 edit_tool_wire_multiple_matches 替代
- `src/tools/edit/planner.rs::parse_rejects_replace_all_and_expected_hash` — planner 内部，工具级 schema/行为测试替代
- `src/tools/edit/planner.rs::plan_one_empty_new_deletes` — planner 内部，工具级 schema/行为测试替代
- `src/tools/edit/planner.rs::plan_one_unique_and_multiple` — planner 内部，工具级 schema/行为测试替代
- `src/tools/edit/python_indent.rs::multiple_exact_rejects_without_replace_all` — 同上
- `src/tools/edit/python_indent.rs::tabs_vs_spaces_is_whitespace_only_reject` — 与 two_space 冗余
- `src/tools/edit/tests.rs::edit_declares_a_thirty_second_timeout` — 超时常量声明
- `src/tools/edit/tests.rs::edit_tool_rejects_utf16` — 与 nul 拒绝同类，留一条
- `src/tools/glob.rs::matcher_supports_braces` — 第三方 glob 语法，非契约
- `src/tools/grep.rs::a_path_outside_the_mode_is_reported_not_panicking` — 仅断言不 panic
- `src/tools/kill.rs::permission_is_fixed_allow_without_command_policy` — 声明
- `src/tools/lsp.rs::content_modified_surfaces_warning_on_data_result` — 警告附加，非主契约
- `src/tools/lsp.rs::finish_locations_relative_inside_absolute_outside` — 相对化细节（列回归）
- `src/tools/lsp_feedback.rs::failed_stays_silent` — "保持沉默"类，非核心
- `src/tools/lsp_feedback.rs::uncovered_extension_stays_silent_even_when_warm` — "保持沉默"类，非核心
- `src/tools/lsp_feedback.rs::warming_stays_silent` — "保持沉默"类，非核心
- `src/tools/read.rs::glob_over_limit_notes_truncation_and_read_consumes_path` — 链式细节
- `src/tools/read.rs::ide_read_checks_cancel_between_chunks` — 与 cancelled_read 冗余
- `src/tools/read.rs::read_declares_a_thirty_second_timeout` — 超时常量声明
- `src/tools/read.rs::read_page_size_ignores_token_budget` — 与默认页冗余
- `src/tools/run.rs::cancel_before_spawn_rejects` — run_lock_gate 版本已守
- `src/tools/run.rs::validate_rejects_empty_command` — 入参细节
- `src/tools/run.rs::wait_yields_so_cancel_can_run` — **测试体零断言**（`let _ = (result, jobs)`），不守任何行为
- `src/tools/run_lock_gate.rs::cancel_before_spawn_releases_when_the_call_ends` — 与 dropped_in_flight 冗余（列回归）
- `src/tools/run_lock_gate.rs::short_command_returns_exit_code_in_foreground` — 与 run::fast_command 冗余（列回归）
- `src/tools/session_search.rs::compacted_history_below_kept_from_seq_remains_searchable` — session 域行为
- `src/tools/session_search.rs::rejects_empty_query` — 入参细节
- `src/tools/wait.rs::unknown_id_is_not_found` — kill 版已守
- `src/tools/webfetch.rs::gate_hard_ssrf_uses_permission_denied_code` — 与 private_ip_is_hard_permission_denied 冗余
- `src/tools/write.rs::external_all_write_skips_workspace_change` — IDE 通知细节
- `src/tools/write.rs::test_sensitive_path_detection` — 谓词单元，端到端由 C4/C14 覆盖
- `src/tools/write.rs::test_write_atomic` — 只断言读回一致，不验证原子性（名不副实）
- `src/tools/write.rs::write_create_only_with_expected_hash_on_missing_file_is_invalid_input` — 入参组合细节
- `src/tools/write.rs::write_declares_a_thirty_second_timeout` — 超时常量声明
- `tests/code_surface.rs::a_second_binding_argument_is_invalid_input_and_does_not_write` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::builtin_code_tools_pass_admission_and_edit_write_join_the_code_surface` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::code_admission_rejections_are_in_the_catalog_payload` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::code_surface_is_projected_without_an_env_gate` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::deep_run_code_return_survives_the_host_process` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::direct_and_nested_calls_share_plan_touch` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::direct_call_of_code_tool_is_routed_before_authorization` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::edit_and_write_advertise_path_on_the_code_surface` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::edit_and_write_succeed_through_run_code_with_path` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::fake_code_tool_joins_the_same_projection_as_builtins` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::historical_file_path_is_rejected_and_does_not_write` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::nested_and_direct_calls_note_the_same_paths` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::nested_write_is_visible_to_the_same_turn_level_effects` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_surface.rs::pipeline_direct_call_uses_the_shared_file_note` — 初稿把 tests/ 440 条整体判守护；此条经读体为投影/路径/plan 细节或与 C10/C11 冗余
- `tests/code_tool_runtime.rs::code_tool_parallelizes_safe_calls_and_rejects_a_second_argument` — 与 C9 冗余
- `tests/code_tool_runtime.rs::code_tool_runs_real_bundle_with_data_only_results` — 与 C9 冗余
- `tests/f2_lock_scope.rs::concurrent_commit_partial_does_not_lose_updates` — session/manager 锁，非 Tools 域（应归 session 链）
- `tests/f2_lock_scope.rs::session_mutation_does_not_hold_records_lock` — session/manager 锁，非 Tools 域（应归 session 链）
- `tests/mcp_integration.rs::block_on_hub_from_turn_runtime_does_not_panic` — 仅断言不 panic
- `tests/mcp_integration.rs::list_tools_returns_mock_catalog` — 形状/冗余
- `tests/mcp_integration.rs::pool_restart_replaces_live_client` — 形状/冗余
- `tests/mcp_integration.rs::turn_runtime_start_list_and_tool_call` — 形状/冗余
- `tests/permission_pipeline.rs::authorize_schema_missing_content_denied` — schema 拒绝与 C12 冗余
- `tests/permission_pipeline.rs::none_tools_always_allow` — 平凡
- `tests/permission_pipeline.rs::subagent_ask_becomes_deny` — 与 engine 版冗余

## 3. 回归测（32，引用均落在测试体/测试注释或对应实现修复点）

- `src/permission/grants.rs::wp081_legacy_q0_free_text_always_true` — WP-081（ask_user 自由文本恒在）
- `src/tool/registry.rs::core_bindings_produce_full_builtin_list` — WP-197（默认绑定无旧 shell，有 run/wait/kill）、WP-057（session_search 门）
- `src/tools/code_search.rs::indexing_wait_message_reads_job_json` — WP-143
- `src/tools/code_search.rs::warm_stale_is_searchable_without_warm_wait` — WP-057/060/061
- `src/tools/custom.rs::child_ignoring_stdin_is_not_a_broken_pipe_failure` — WP-018（Broken pipe）
- `src/tools/custom.rs::finished_reaps_grandchild_holding_stdout` — ISSUE-039 X1/X2
- `src/tools/custom.rs::finished_kills_orphaned_grandchild_before_side_effect` — ISSUE-039 X2
- `src/tools/lsp.rs::index_not_ready_rejects_unavailable_for_symbols_and_diagnostics` — WP-206 ③
- `src/tools/lsp.rs::finish_locations_relative_inside_absolute_outside` — WP-206 ⑧
- `src/tools/code/render.rs::dropped_undefined_line_sits_in_the_head` — WP-206 #7（实现侧注释，测试名对应）
- `src/tools/read.rs::test_read_start_line_past_end_resolves_empty` — WP-206 ④（commit 1350f56）
- `src/tools/read.rs::test_read_empty_file_past_end_resolves_empty` — WP-206 ④（commit 1350f56）
- `src/tools/plan.rs::test_create_rejected_does_not_create_file` — WP-050 T3
- `src/tools/run.rs::fast_command_returns_exit_code` — WP-197（仅 echo 标记串 hello-wp197，弱引用）
- `src/tools/run_lock_gate.rs::cancel_before_spawn_releases_when_the_call_ends` — WP-206 A 组（模块头）
- `src/tools/run_lock_gate.rs::dropped_in_flight_future_releases_and_panic_unwind_releases` — WP-206 A 组
- `src/tools/run_lock_gate.rs::four_parallel_write_runs_finish_in_about_one_second` — WP-206 A 组（id wp206-a）
- `src/tools/run_lock_gate.rs::locks_release_after_spawn_while_command_still_runs` — WP-206 A 组
- `src/tools/run_lock_gate.rs::short_command_returns_exit_code_in_foreground` — WP-206 A 组
- `src/tools/run_lock_gate.rs::spawn_failure_releases_locks` — WP-206 A 组
- `src/tools/run_lock_gate.rs::window_is_capped_by_run_code_deadline` — WP-206 A 组
- `src/tools/subagent/hub.rs::hub_lagged_rescan_dedups_per_child` — ISSUE-054
- `src/tools/subagent/spawn_contract.rs::child_calls_agent_provider_endpoint_not_parent_provider` — parent-provider clobber 修复回归（注释明示）
- `src/tools/webfetch.rs::ssrf_rejects_loopback_and_private_ips` — ISSUE-012（IPv4-mapped）、WP-203（100.64 仍拒）
- `src/tools/webfetch.rs::ssrf_allows_benchmarking_fake_ip_range` — WP-203（198.18/15 放行）
- `src/tools/webfetch.rs::ssrf_allows_hostname_resolved_to_fake_ip` — WP-203
- `src/mcp/client.rs::r1_skips_notification_before_response` — ISSUE-034 R1（实现注释）
- `src/mcp/client.rs::r2_hang_times_out_and_marks_dirty` — ISSUE-034 R2
- `src/mcp/client.rs::r3_cancel_marks_dirty_next_client_ok` — ISSUE-034 R3
- `tests/wp028_probe_s1_xdev_move.rs::probe_s1_cross_device_overwrite_succeeds` — WP-040 / ISSUE-038
- `tests/wp028_probe_s2_create_race.rs::probe_s2_create_file_race` — WP-040 / ISSUE-037
- `tests/wp028_probe_s2_create_race.rs::probe_s2_write_file_bytes_no_overwrite_race` — WP-040 / ISSUE-037

未计入：`ask_user.rs` 的 WP-050 T2 注释在 helper `line_for` 上；`code_search` agent_view_* 无测试内引用；`baseline` 的 WP-198 注释在 helper `assert_kind`；`custom.rs::timeout_kills_grandchild_process` 属进程组修复但无引用。

## 4. 其余非核心（429，按类别计数）

- 文件工具细节（read/grep/glob/write/edit/file_path/coords/snippet/file_identity）：102
- 执行器/schema/输出/信号/outcome/registry/availability：75
- 会话与产品工具（subagent/todo/plan/ask_user/knowledge/litecode_workspace）：91
- 检索（code_search/session_search/sync 行）：37
- run_code 渲染/manifest/baseline 及 code_surface、code_tool_runtime 集成：34
- 命令（run/wait/kill/custom/custom_hub/bash_safety）：31
- 网络（webfetch/websearch）：21
- 权限（文案/预设/匹配器/冗余管线）：19
- LSP（lsp/lsp_feedback）：9
- MCP（集成形状/mcp_tool 命名）：5
- 资源锁基本用例（write_lock）：3
- 非 Tools 域（f2_lock_scope）：2
