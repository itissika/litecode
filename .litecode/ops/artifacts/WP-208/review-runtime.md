# WP-208 Runtime 域核心链路人工核对（基线 3acfe31，tip 28d3c85 测试一致）

> 只读读码判断，未跑 cargo、未改/删测试、未 commit。范围：src/ptc、src/runtime、src/terminal、src/agent、src/proc_lifetime.rs + 指定 16 个 tests/ 文件。
> 范围内测试总数 278（src 173：ptc 58 / runtime 78 / terminal 35 / agent 2 / proc_lifetime 0；tests/ 105）。
> 结论：17 条核心链路，守护测试 122 条；其余 156 条非核心。

## 1. 核心链路 → 守护测试

### C1 PTC 值边界：JS→JSON 时 undefined 在参数中静默丢弃、在结果中剥离/置 null 并报 path（≤10 条 +N more），bigint/symbol/非安全整数带 path 硬错且不触达宿主
依据：src/ptc/boundary.rs:64-103（Undefined 策略与 dropped 行）、:177-200（undefined/bigint/symbol 判定）、:346 path_at
- src/ptc/tests.rs::undefined_in_result_is_stripped_or_nulled_and_reported
- src/ptc/tests.rs::undefined_in_arguments_is_dropped_silently
- src/ptc/tests.rs::dropped_undefined_list_is_capped_with_more_count
- src/ptc/tests.rs::illegal_output_values
- src/ptc/tests.rs::illegal_arguments_skip_host
- src/ptc/tests.rs::host_integer_above_safe_range_is_rejected

### C2 PTC 预算/取消：死循环被 interrupt 打断；等待宿主/审批时暂停计费；取消时 in-flight 调用被取消，grace 后未结束调用记为未完成；成功返回也取消残留调用，未 await 调用被等待并记账
依据：src/ptc/budget.rs:82-116（pause guard）、src/ptc/engine.rs:78（interrupt）、src/ptc/calls.rs:118 seal(grace)、engine.rs:166/327
- src/ptc/tests.rs::budget_interrupts_infinite_loop
- src/ptc/tests.rs::budget_pause_is_not_charged
- src/ptc/tests.rs::cancel_while_js_running_cancels_inflight
- src/ptc/tests.rs::cancel_while_awaiting_host
- src/ptc/tests.rs::cancel_grace_records_unfinished
- src/ptc/tests.rs::success_cancels_inflight_calls
- src/ptc/tests.rs::unawaited_calls_are_waited_and_recorded

### C3 PTC 成功不被改写：程序已 return 后 drain 遇超时/取消保留返回值，其他 drain 失败才替换成功
依据：src/ptc/engine.rs:316-326（drive_queued→reconcile_drain）、:440-454
- src/ptc/tests.rs::success_survives_budget_expiry_at_return
- src/ptc/engine.rs::success_is_not_rewritten_when_the_clock_is_already_exhausted
- src/ptc/engine.rs::timeout_during_drain_keeps_the_returned_value
- src/ptc/engine.rs::cancel_during_drain_keeps_the_value
- src/ptc/engine.rs::other_drain_failures_replace_success

### C4 PTC 栈/内存封口：栈溢出与 OOM 被 JS catch 后仍封为 ResourceLimit（Stack/Memory），不靠消息文本；超大稀疏数组分配前拒绝；深嵌套不崩
依据：src/ptc/engine.rs:1205-1221 seal_latched_stack、:1223 note_stack latch、:1293 native_stack_bytes、:172-210 LimitAlloc 内存 fault
- src/ptc/tests.rs::stack_limit
- src/ptc/tests.rs::stack_overflow_stays_sealed_when_the_program_catches_it
- src/ptc/tests.rs::stack_overflow_caught_then_immediate_return_is_still_sealed
- src/ptc/tests.rs::memory_limit
- src/ptc/tests.rs::memory_exhaustion_stays_sealed_when_the_program_catches_it
- src/ptc/tests.rs::caught_allocation_failure_is_memory_when_the_stack_watch_is_disarmed
- src/ptc/tests.rs::sparse_huge_array_rejected_before_allocation
- src/ptc/tests.rs::deep_nesting_does_not_crash

### C5 PTC 沙箱：无 fetch/require/process/setTimeout/std/os/Atomics/SAB，import 失败
依据：src/ptc/tests.rs:815-838 断言集；engine 只注入 host 调用面
- src/ptc/tests.rs::no_ambient_capabilities

### C6 工具错误语义：ToolError 可 catch 且 instanceof；未 catch 保留 error_code 并点名调用；用户异常不按消息分类；第二参数为可捕获 TypeError；caught 错误计数不列出
依据：src/ptc/engine.rs:494（FailureKind 分类）、tools/code 渲染
- src/ptc/tests.rs::tool_error_is_catchable_and_instanceof
- src/ptc/tests.rs::uncaught_tool_error_keeps_code
- src/ptc/tests.rs::user_exceptions_are_not_classified_by_message
- tests/run_code_model_view.rs::caught_tool_error_is_counted_and_not_listed
- tests/run_code_model_view.rs::uncaught_tool_error_names_the_call
- tests/run_code_model_view.rs::a_second_argument_is_a_catchable_type_error

### C7 输出预算：log 保前缀、最终值计入预算、截断只保可容纳前缀；进模型请求的结果受预算约束，超时保留 log 尾并点名预算
依据：src/ptc/output.rs push（:81 测试对应实现）、engine.rs:388 输出 ceiling
- src/ptc/tests.rs::output_limit_keeps_log_prefix
- src/ptc/tests.rs::output_limit_counts_final_value
- src/ptc/output.rs::push_keeps_the_fitting_prefix
- src/ptc/tests.rs::dense_array_within_output_budget
- tests/run_code_model_view.rs::large_result_is_budgeted_in_the_request
- tests/run_code_model_view.rs::timeout_keeps_the_log_tail_and_names_the_budget

### C8 Agent loop：纯文本即停；function_call 先于 output 落盘；max_steps 封顶；取消/流中断时已有输出落盘、未执行工具补 interrupted pad；incomplete 调用不执行；持久化失败回滚；auto compact 失败/取消不调模型
依据：src/agent/core.rs:22-62（取消/compact/中断）、:120-162（skip_tools+pads）、:184-195 persist_or_stop
- tests/agent_loop.rs::single_text_response_stops_loop
- tests/agent_loop.rs::tool_step_persists_function_call_before_output
- tests/agent_loop.rs::max_steps_with_perpetual_tools
- tests/agent_loop.rs::cancelled_after_model_persists_output
- tests/agent_loop.rs::cancelled_after_stream_skips_tool_execution
- tests/agent_loop.rs::incomplete_function_call_is_not_executed
- tests/agent_loop.rs::persist_failure_rolls_back_working_set
- tests/agent_loop.rs::auto_compact_fail_stops_turn_without_model_call
- tests/agent_loop.rs::cancel_during_auto_compact_is_cancelled_without_model_call
- tests/agent_e2e_responses.rs::agent_e2e_incomplete_seals_text_to_disk

### C9 流式投影：流式 item 落到它打开的行；未确认 item 封 incomplete；provider 复用 id 开新行；身份冲突/终止重复 id fail-closed；写失败使调用失败但仍封口
依据：src/runtime/stream_projection.rs:223 settle、:475 settle_open
- src/runtime/stream_projection.rs::a_streamed_item_settles_into_the_row_it_opened
- src/runtime/stream_projection.rs::an_item_the_call_never_confirmed_settles_incomplete
- src/runtime/stream_projection.rs::a_call_that_reuses_a_provider_id_opens_a_new_row
- src/runtime/stream_projection.rs::same_call_identity_conflicts_fail_closed
- src/runtime/stream_projection.rs::terminal_duplicate_id_fails_closed
- src/runtime/stream_projection.rs::a_write_the_session_refuses_fails_the_call
- src/runtime/stream_projection.rs::update_stream_item_failure_fails_settle_and_still_seals

### C10 请求前缀稳定（缓存）：已发出的请求前缀在 turn 内及跨 turn 重载不被改写；reminder 只追加；code 模式工具表稳定；compact 后恢复行位置正确
依据：src/runtime/exec.rs:128 prepare_view、:138-160 sync_request_seam 追加式写入、:183 persist_new
- tests/turn_prefix_stability.rs::tool_loop_llm_request_prefix_json_is_stable
- tests/turn_prefix_stability.rs::code_mode_tool_list_prefix_stays_stable_across_the_turn
- tests/turn_prefix_stability.rs::job_exit_mid_turn_does_not_rewrite_already_sent_prefix
- tests/turn_prefix_stability.rs::persist_item_reused_message_id_must_not_rewrite_final_row
- tests/turn_prefix_stability.rs::persist_item_reused_function_call_id_must_not_rewrite_final_row
- tests/turn_prefix_stability.rs::next_turn_reload_must_keep_sent_row_when_provider_reuses_id
- tests/turn_prefix_stability.rs::state_reminders_only_append_across_steps
- tests/turn_prefix_stability.rs::restore_row_lands_after_compacted_and_prefix_stays

### C11 排队消息/steer/stop_and_send：pending 在 tool loop seam 注入，最终回复期间到达则延长 turn；queue 模式不在 seam 消费、turn 结束后 FIFO 每条一 turn；steer 注入当前 turn；stop_and_send 取消后起新 turn（含启动窗口）；空闲 flush 只落一条 user 行
依据：src/runtime/exec.rs:138-172（claim_pending_messages_for_turn/restore）、src/agent/core.rs:146-152（has_pending→continue）、src/runtime/pending_auto_turn.rs:48 try_begin_pending_flush
- tests/pending_messages.rs::queued_message_is_injected_at_the_tool_loop_seam
- tests/pending_messages.rs::queued_message_during_final_response_extends_the_turn
- src/agent/core.rs::stop_check_continues_once_for_a_queued_message
- tests/wp098_follow_up_queue.rs::runtime_queue_not_drained_at_step_seam
- tests/wp098_follow_up_queue.rs::runtime_steer_still_injects_at_step_seam
- tests/wp098_follow_up_queue.rs::runtime_stop_and_send_cancels_then_runs_new_turn_no_notice
- src/runtime/pending_auto_turn.rs::idle_flush_reserves_claims_and_persists_one_user_row
- src/runtime/pending_auto_turn.rs::busy_session_leaves_the_queue_alone
- src/runtime/pending_auto_turn.rs::runtime_queue_multiple_runs_one_turn_each_fifo
- src/runtime/pending_auto_turn.rs::runtime_stop_and_send_starting_window

### C12 自动 turn 唤醒：无订阅者不预留、忙时留 mailbox、UI kill 后空闲再提醒；子会话自动 turn 一律 deny sink（不得继承上次审批 sink）；子退出触发父 reminder
依据：src/runtime/bash_auto_turn.rs、src/runtime/subagent_auto_turn.rs:29-180、src/runtime/pending_auto_turn.rs:163-240
- src/runtime/bash_auto_turn.rs::idle_without_subscribers_does_not_reserve
- src/runtime/bash_auto_turn.rs::busy_session_leaves_mailbox
- src/runtime/bash_auto_turn.rs::ui_kill_while_busy_prepares_user_stopped_reminder_once_idle
- src/runtime/subagent_auto_turn.rs::child_uses_deny_sink_regardless_of_last_sink
- src/runtime/pending_auto_turn.rs::child_uses_deny_sink_regardless_of_last_sink
- tests/subagent_session_isolation.rs::child_exit_triggers_parent_auto_turn_reminder

### C13 子会话生命周期/隔离：launch 建持久子会话并挂父链接；父事件通道不收子事件；绑定失败/缺 call_id 不留孤儿；删父级联；子 turn 读自身会话配置、身份不可被 set_primary 切换；结果从持久 turn 读取
依据：src/tools/subagent/launch.rs:249；tests/architecture_layers.rs:102 约束 runtime 不开子会话
- tests/subagent_session_isolation.rs::subagent_launch_creates_durable_child_with_parent_link
- tests/subagent_session_isolation.rs::parent_event_channel_does_not_receive_child_turn_events
- tests/subagent_session_isolation.rs::failed_binding_aborts_orphan_child_session
- tests/subagent_session_isolation.rs::missing_call_id_scope_errors_without_creating_child
- tests/subagent_session_isolation.rs::remove_parent_cascades_child
- tests/subagent_child_turn_contract.rs::child_turn_reads_config_from_its_own_session_row
- tests/subagent_child_turn_contract.rs::child_identity_is_not_switchable_by_set_primary
- tests/subagent_child_turn_contract.rs::completed_child_result_is_read_from_the_durable_turn

### C14 子代理并发：并发 send 只有一个赢；忙时排队不起 turn；拒绝未知/他人 id；send 后 stop 取消新 turn；wait 报 user stop；父取消不停后台子、stop 工具可停；worker panic 也释放会话；两 launch 并发
依据：src/tools/subagent/send.rs:39 call_send、:133 deliver_to_busy；stop.rs
- tests/subagent_send.rs::concurrent_sends_only_one_wins
- tests/subagent_send.rs::send_to_busy_child_queues_without_starting_a_turn
- tests/subagent_send.rs::send_rejects_unknown_and_foreign_ids
- tests/subagent_send.rs::send_then_stop_cancels_the_new_turn
- tests/subagent_send.rs::wait_returns_cancelled_when_user_stopped
- tests/subagent_tool_pipeline.rs::parent_cancel_does_not_stop_background_child_and_stop_tool_can
- tests/subagent_tool_pipeline.rs::worker_panic_finishes_job_and_releases_session
- tests/subagent_tool_pipeline.rs::pipeline_runs_two_launches_concurrently

### C15 agent bash 作业：前台超时 promote 不杀、promote 后退出进 mailbox；前台取消杀进程且不进 mailbox；嵌套取消后 detach 续读；未 detach 结束不发 mailbox/handler；UI kill 杀 pid 并标 user_killed；wait 超时作业仍活；tee 超上限截断标记；订阅时水合作业、kill 后清空
依据：src/terminal/agent_jobs.rs:795（超时 promote 不杀）、:847（Exited 无 mailbox）、:685 kill_registered、:606 take_notice_from_mailbox；src/terminal/tee.rs
- src/terminal/agent_jobs.rs::foreground_timeout_promotes_without_killing
- src/terminal/agent_jobs.rs::foreground_cancel_kills_and_skips_mailbox
- src/terminal/agent_jobs.rs::detached_read_continues_after_nested_cancel
- src/terminal/agent_jobs.rs::undetached_finish_skips_mailbox_and_exit_handler
- src/terminal/agent_jobs.rs::kill_registered_kills_pid_and_marks_user_killed
- src/terminal/agent_jobs.rs::wait_async_times_out_while_job_stays_alive
- src/terminal/tee.rs::file_stops_at_max_and_flags_truncation
- tests/subscribe_bash_hydration.rs::subscribe_hydrates_bash_jobs_and_clears_after_kill

### C16 终端安全/隔离：agent kill 拒绝人类 PTY 与猜测 id；id 为不可猜 uuid；WS 终端按 owner 隔离、断开清理；PTY 交互回显；跨块 CJK 不撕裂
依据：src/terminal/mod.rs:470-496（kill/kill_from_ui/is_human_pty）、:20 Uuid；src/terminal/pty.rs drain_utf8
- src/terminal/mod.rs::kill_rejects_human_pty_id
- src/terminal/mod.rs::kill_unknown_or_guessed_id_is_rejected
- src/terminal/mod.rs::ids_are_unguessable_uuid_and_distinct
- tests/terminal_ws_isolation.rs::websocket_terminals_are_owner_isolated_and_disconnect_cleaned
- tests/terminal_pty_contract.rs::interactive_echo_round_trip
- src/terminal/pty.rs::drain_utf8_keeps_a_cjk_character_split_across_chunks

### C17 架构/启动死名单（源码静态约束）：分层不越界；PTY 符号仅在 terminal 模块、前端无 node-pty；所有子进程 spawn 等 sweep gate、gate 无超时、sweep 先于 ready 且不内联
依据：tests/architecture_layers.rs:56-114；tests/terminal_bypass_death_list.rs:52-110；tests/startup_order_death_list.rs:71-134；src/proc_lifetime.rs:48 sweep_orphan_lsp_processes
- tests/architecture_layers.rs::session_layer_does_not_absorb_the_hub
- tests/architecture_layers.rs::tool_layer_does_not_finish_or_join_turns
- tests/architecture_layers.rs::tool_turn_helper_uses_human_reserve_then_spawn
- tests/architecture_layers.rs::runtime_and_protocol_do_not_open_child_sessions
- tests/architecture_layers.rs::tool_core_does_not_own_subagent_completion_delivery
- tests/terminal_bypass_death_list.rs::death_list_src_pty_symbols_only_in_terminal_module
- tests/terminal_bypass_death_list.rs::death_list_no_node_pty_in_desktop_or_web_package_json
- tests/terminal_bypass_death_list.rs::death_list_no_node_pty_require_in_desktop_web_src
- tests/startup_order_death_list.rs::every_spawn_waits_for_the_sweep_gate
- tests/startup_order_death_list.rs::sweep_gate_has_no_timeout
- tests/startup_order_death_list.rs::serve_run_arms_sweep_before_ready_path
- tests/startup_order_death_list.rs::serve_run_does_not_sweep_inline

**缺口（只记录，不改）**：src/proc_lifetime.rs 0 单测（bind_child_to_parent / Windows job kill-on-close / sweep_once 三平台实现无行为测试，仅 C17 静态约束间接覆盖）。

## 2. 对初稿的修正

### 补（初稿漏掉，37 条测试 + 6 条链路）
- 链路级：初稿把范围内 tests/ 105 条整体塞进“集成 440 全列守护”，未成链路；本稿新立 C8 agent loop、C9 流式投影（初稿完全没有 runtime/stream_projection）、C10 前缀稳定、C11 排队/steer、C12 自动 turn、C13/C14 子代理。
- C1 +3：illegal_output_values、illegal_arguments_skip_host、host_integer_above_safe_range_is_rejected —— 初稿标题写 bigint/symbol 带 path 报错，但所列 4 条全是 undefined，没有守 bigint/symbol/非安全整数（boundary.rs:197-200）。
- C2 +1：unawaited_calls_are_waited_and_recorded（未 await 调用账目）。
- C3 +2：engine.rs::success_is_not_rewritten_when_the_clock_is_already_exhausted、other_drain_failures_replace_success —— reconcile_drain（engine.rs:443）的两条分支，初稿只列了 2 条保值分支，缺“其他失败替换成功”反向分支。
- C4 +1：deep_nesting_does_not_crash（原生栈保护的直接守护）。
- C5 +1：no_ambient_capabilities（沙箱底线，初稿缺）。
- C6 +3：tool_error_is_catchable_and_instanceof、uncaught_tool_error_keeps_code、user_exceptions_are_not_classified_by_message。
- C7 +3：output_limit_keeps_log_prefix、output_limit_counts_final_value、output.rs::push_keeps_the_fitting_prefix（初稿 run_code 渲染链只放了 tools/ 侧）。
- C9 +7：stream_projection 7 条（见上）。
- C11 +5：agent/core.rs::stop_check_continues_once_for_a_queued_message；pending_auto_turn 的 idle_flush_reserves_claims…、busy_session_leaves_the_queue_alone、runtime_queue_multiple_runs_one_turn_each_fifo、runtime_stop_and_send_starting_window。
- C12 +5：bash_auto_turn ×3、subagent_auto_turn::child_uses_deny_sink…、pending_auto_turn::child_uses_deny_sink…（后者初稿仅在回归节）。
- C15 +4：undetached_finish_skips_mailbox_and_exit_handler、kill_registered_kills_pid_and_marks_user_killed、wait_async_times_out_while_job_stays_alive、tee::file_stops_at_max_and_flags_truncation。
- C16 +2：ids_are_unguessable_uuid_and_distinct、pty::drain_utf8_keeps_a_cjk_character_split_across_chunks。

### 剔（初稿误判，4 条按名误判 + 45 条集成测试降为非核心）
- exception_includes_stack、exception_keeps_stack、syntax_error_on_the_second_line_keeps_stack_and_location：初稿因名含 “stack” 归入“栈溢出封口”，实为 JS 异常堆栈文本/位置渲染（tests.rs:553/568/623），与栈溢出封口无关 → 非核心（诊断渲染）。
- host_receives_extra_arguments_as_a_json_array：初稿归“undefined 边界”，实为宿主参数打包形状；第二参数语义已由 C6 a_second_argument_is_a_catchable_type_error 守 → 非核心（保留为回归）。
- 归类修正（不计剔）：dense_array_within_output_budget 初稿放在“预算/取消”链，实测输出预算，移到 C7。
- 初稿“集成 440 全列守护”中范围内 105 条，经读码仅 60 条入链，其余 45 条降为非核心，主要类型：schema/字段形状（launch_schema_exposes_only_team_assignment_fields、send_schema_requires_id_and_message、launch_ignores_model_and_max_steps_input、turn_options_carry_identity_only）、tools 域入参校验（run_code_model_view 的 start_line_zero/file_path_alias/parent_glob/missing_file_message/partial_edit，归 Tools 域）、渲染外观（small_result_is_verbatim、cjk_result_does_not_panic、syntax_error_shows_the_source_line、exception_shows_boom…）、happy path 与 C8 重复（function_call_then_text_grows_transcript、cancelled_breaks_loop、agent_e2e_responses_text_only/tool_loop_hits_max_steps、responses_fixtures_are_not_chat_dialect）、广播/列表展示（child_lifecycle_is_broadcast_to_workspace、subagent_list_reports_raw_session_statuses、subagent_bound_arrives_on_parent…）、启动死名单中与 Runtime 无关的 LSP/索引项（lsp_warmup_runs_no_subprocess、first_need_paths_probe_before_spawning、no_language_specific_branch_added、attach_does_not_touch_legacy_text_index_inline、serve_run_does_not_warm_optional_engines）等。

## 3. 回归测试（范围内全列，按测试体/注释中 WP-/ISSUE- 引用 + 本域近期修复）
- tests/agent_loop.rs::cancel_during_auto_compact_is_cancelled_without_model_call — ISSUE-033
- tests/subagent_send.rs::send_to_busy_child_queues_without_starting_a_turn — ISSUE-053, WP-092, WP-100
- tests/subagent_send.rs::concurrent_sends_only_one_wins — ISSUE-053, WP-092
- tests/wp098_follow_up_queue.rs::runtime_main_session_pending_still_injects_at_seam — WP-098
- tests/wp098_follow_up_queue.rs::runtime_steer_still_injects_at_step_seam — WP-098
- tests/wp098_follow_up_queue.rs::runtime_queue_not_drained_at_step_seam — WP-098
- tests/wp098_follow_up_queue.rs::runtime_stop_and_send_cancels_then_runs_new_turn_no_notice — WP-098
- tests/wp098_follow_up_queue.rs::stop_and_send_cancels_then_runs_follow_up_without_waking_parent — WP-098
- tests/startup_order_death_list.rs::serve_run_does_not_sweep_inline — WP-165
- src/ptc/tests.rs::host_receives_extra_arguments_as_a_json_array — WP-206
- src/ptc/tests.rs::undefined_in_result_is_stripped_or_nulled_and_reported — WP-206
- src/runtime/observer.rs::turn_end_reason_log_schema_is_snake_case_for_every_variant — WP-069
- src/runtime/observer.rs::turn_error_kind_serializes_snake_case — WP-069
- src/runtime/exec.rs::body_bytes_reflects_instructions — WP-090
- src/runtime/pending_auto_turn.rs::child_uses_deny_sink_regardless_of_last_sink — WP-098
- （补，无文内引用但由提交引入）src/ptc/tests.rs::stack_overflow_caught_then_immediate_return_is_still_sealed — WP-207 G1 / 99ac65f
- （补）src/ptc/tests.rs::dropped_undefined_list_is_capped_with_more_count、undefined_in_arguments_is_dropped_silently — WP-206 D / 61cfd38
- （补）src/ptc/engine.rs::timeout_during_drain_keeps_the_returned_value、cancel_during_drain_keeps_the_value — drain 保值修复
共 15 条文内引用（与初稿一致）+ 5 条按提交补。

## 4. 非核心测试按类别计数（156）
| 类别 | 数量 |
|---|---|
| ptc 其余（返回值/日志级别/console 格式/libs 样例/seq/in_flight/并发 host/微任务/嵌套 round-trip/cold_start 性能/json&boundary 内部等价/syntax_error/JS 异常堆栈渲染） | 24 |
| runtime llm_resolve（模型能力/凭证/选择修复） | 13 |
| runtime exec（停止判定、body_bytes、thinking 回填与 toast 级别） | 10 |
| runtime pending_auto_turn 其余（图片顺序、mentions/snapshot reminder、revert 后不投递、pending 先于 follow-up、空队列等） | 9 |
| runtime phase 状态机事件 | 6 |
| runtime stream_projection 其余 | 5 |
| runtime diagnostics 落盘 | 4 |
| runtime observer 序列化 | 4 |
| runtime mod（snapshot/错误映射） | 4 |
| runtime custom_auto_turn | 3 |
| runtime subagent_auto_turn / bash_auto_turn 其余 | 4 |
| terminal agent_jobs 其余（wait yield、wire snapshot、tail 视图、id 分配、jobs_changed 等） | 9 |
| terminal tee 其余 / ansi / shell / pty 其余 / mod 其余 | 4+3+4+2+2=15 |
| agent/core.rs::stop_check_breaks_without_pending | 1 |
| tests/ 集成非核心（见 §2 剔） | 45 |
| **合计** | **156** |
