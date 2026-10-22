# WP-208 review-session（基线 3acfe31）

> 只读核对：HEAD=28d3c85，`git diff 3acfe31 HEAD -- src tests` 为空，故直接读工作树即等于基线。未跑 cargo、未改 src/tests。
> 范围内测试总量：src 单测 473（session/client_protocol/serve/config/cli/telemetry/types/authority/version）+ tests/ 集成 112。

## 结论摘要
- 初稿把范围内 **全部 112 个集成测试** 无差别塞进“集成 tests/”核心段（脚本按文件收录，非判断），同时 src 侧只收 81 条且集中在 sqlite/manager/connection/project，**漏掉** serve 绑定、WS 鉴权单测、pending 断线≠Deny（WP-186）、set-model 原子性、global_db migrate/seed、settings_writer 回滚、delete/tombstone、snapshot restore 等整条链路。
- 本稿定 **14 条核心链路**，共约 70 个守护测试（必要且足够）；补 30 条、剔 ~150 条（112 集成里约 75 降为非核心 + src 侧 ~70 条冗余/细节）。
- 无守护缺口：CLI 主入口/参数解析、authority 业务语义、telemetry 上报链路、version 仅冒烟；set-model 时 in-flight turn 处理无测试（与 WP-189 dig 结论一致）。

## 核心链路与守护测试
### C1 Session 数据层事务：创建/追加/删除幂等、乐观并发、故障回滚
- 守的行为：每次变更走单事务，operation_id 幂等、stale revision 冲突、commit 前故障整体回滚、commit 后故障可幂等重放。
- 守护测试：`tests/session_data_crud.rs::create_append_meta_delete`、`tests/session_data_crud.rs::idempotent_operation_id_returns_same_receipt`、`tests/session_data_crud.rs::conflict_on_stale_revision`、`tests/session_data_crud.rs::parent_delete_cascades_child`、`tests/session_data_fault.rs::fault_before_commit_rolls_back`、`tests/session_data_fault.rs::fault_after_commit_keeps_rows_and_operation_is_idempotent`、`tests/session_data_concurrency.rs::same_revision_double_append_one_conflict`、`tests/session_data_concurrency.rs::append_then_delete_late_commit_conflicts`、`src/session/data/sqlite/ops.rs::tests::persist_receipt_deleted_rejects_surviving_session_row`

### C2 seq 单调与高水位（不复用已删 seq）
- 守的行为：next_seq = max+1 持久化，truncate/revert/compact 后追加永不复用已删 seq，seal 不增 seq。
- 守护测试：`src/session/data/sqlite/session.rs::tests::truncate_then_append_does_not_reuse_deleted_seqs`、`…::reopen_after_truncate_keeps_high_water_and_appends_into_the_hole`、`…::in_progress_item_is_retrievable_and_seal_keeps_seq`、`…::load_events_roundtrip_append_origin_is_contiguous_seq`、`src/session/data/sqlite/schema.rs::tests::existing_session_db_backfills_next_seq_to_max_plus_one`、`src/session/data/final_immutability_tests.rs::reopen_after_revert_does_not_reuse_a_deleted_seq`、`tests/wire_seq_identity.rs::revert_then_append_does_not_reuse_deleted_seq`

### C3 final 不可改 + 崩溃封口（冷恢复）
- 守的行为：final 行永不重写；重启时 in-flight 行封为 incomplete、未答 call 补 interrupt 结果，后续追加接在其后。
- 守护测试：`src/session/data/final_immutability_tests.rs::a_finalised_row_is_never_rewritten`、`…::a_restart_seals_rows_the_previous_run_left_in_flight`、`…::a_restart_seals_an_in_flight_call_and_appends_its_interrupt_result`、`…::sealing_a_settled_row_is_refused`、`src/session/data/sqlite/session.rs::tests::a_crashed_turn_is_sealed_incomplete_and_its_continuation_appends`

### C4 compact 检查点与 fold 一致性
- 守的行为：compact 只追加 checkpoint、不删历史 detail；reload 后 fold 结果与增量工作集一致；第二次 compact 追加新 checkpoint。
- 守护测试：`src/session/data/sqlite/session.rs::tests::compact_never_deletes_historical_detail_rows`、`…::writer_working_set_matches_fold_after_append_and_keep_recent_compact`、`…::incremental_surface_matches_fold_after_compact_and_append`、`…::compact_summary_agent_view_is_assistant_and_survives_reload`、`…::revert_retaining_earlier_compact_restores_pointers`、`tests/wire_seq_identity.rs::compact_then_revert_before_checkpoint_drops_compact_and_resets_pointers`、`tests/wire_seq_identity.rs::second_compact_appends_new_seq_and_keeps_both_checkpoints`

### C5 线协议 seq 身份 / snapshot 游标
- 守的行为：wire 只发新 seq 行，snapshot.next_seq == DB max+1，runtime-only 流事件不长 seq、不发 snapshot。
- 守护测试：`tests/wire_seq_identity.rs::step_committed_emits_only_new_seq_and_snapshot_matches_max`、`…::turn_finished_snapshot_next_seq_matches_db_max_plus_one`、`…::runtime_only_stream_events_do_not_emit_snapshot_or_grow_seq`、`…::empty_session_cursor_is_minus_one_zero`、`src/client_protocol/controller/mod.rs::tests::cancel_seal_restamps_changed_seqs_without_next_seq_growth`、`src/client_protocol/controller/mod.rs::tests::revert_then_append_delivers_new_seq_using_persisted_high_water`、`src/client_protocol/protocol.rs::tests::buffer_load_result_is_a_seq_window`

### C6 Turn 生命周期（单 turn 互斥、取消、预约、revert/compact lease）
- 守的行为：同 session 同时仅一 turn；取消在 starting 期也生效；compact/revert 与 turn 互斥；失败回滚 running 与 guard。
- 守护测试：`src/session/manager.rs::tests::begin_turn_rejects_second_while_running`、`…::cancel_during_starting_turn_cancels_on_start`、`…::turn_reservation_closes_pre_spawn_idle_window`、`…::compact_lease_atomically_blocks_turn_and_revert`、`…::revert_during_running_turn_waits_for_finish_and_holds_exclusive_lease`、`…::start_turn_fanout_spawn_failure_rolls_back_running_and_turn_guard`、`…::finish_turn_clears_running_immediately_and_allows_rebegin`

### C7 待发队列（pending messages）
- 守的行为：enqueue/claim 不写 log；仅 running turn 可 claim；revert 丢弃失效队列、失败 revert 保留。
- 守护测试：`src/session/manager.rs::tests::enqueue_remove_and_claim_never_touch_the_log`、`…::claim_only_for_the_running_turn_and_restore_prepends`、`…::a_reverted_log_drops_the_queue_it_made_meaningless`、`…::a_failed_revert_keeps_the_queue`、`tests/session_data_lifecycle.rs::queue_capacity_keeps_overflow_pending_until_cancelled`

### C8 权限往返与 pending_permissions（WP-186）
- 守的行为：grant 按 request_id 精确匹配 allow/deny；断线/cancel 是 abort 而非 Deny；snapshot 恒序列化 pending_permissions，waiting_for_user 由列表长度派生。
- 守护测试：`src/client_protocol/connection.rs::tests::matching_accept_resolves_allow`、`…::matching_reject_resolves_deny`、`…::retain_pending_on_disconnect_does_not_deny`、`…::abort_all_pending_is_aborted_not_deny`、`…::unknown_grant_is_buffered_as_stray_then_consumed`、`…::concurrent_asks_resolve_independently`、`src/client_protocol/project.rs::tests::snapshot_always_serializes_pending_permissions_even_when_empty`、`…::turn_awaiting_permission_derived_from_list_len`、`…::pending_permissions_matches_frozen_shape_fields`、`tests/client_protocol_permission.rs::ws_permission_bridge_abort_on_cancel_is_not_deny`、`tests/client_protocol_permission.rs::ws_bridge_approval_round_trip_with_free_text`

### C9 子会话只读与删除级联（wp091/wp099）
- 守的行为：用户对 child 的 run/retry/enqueue 被拒；删除 running 子/有 running 子的父被拒；删除留 tombstone 并级联。
- 守护测试：`src/client_protocol/connection.rs::tests::wp091_agent_run_on_child_rejected_readonly`、`…::wp091_delete_parent_with_running_child_returns_descendant_running`、`…::wp091_delete_child_tombstone_and_notice`、`src/session/manager.rs::tests::delete_operation_is_blocked_by_active_descendant_turns`、`src/session/delete.rs::tests::tombstone_survives_reload`、`src/session/delete.rs::tests::remove_session_returns_descendants_sorted_root_last`、`tests/session_delete.rs::delete_listed_idle_session_after_cold_start_without_subscribe`

### C10 会话列表/冷启动可见性
- 守的行为：空新会话隐藏，有用户消息的会话在冷启动/坏行/缺 blob 下仍列出。
- 守护测试：`tests/session_list_empty_filter.rs::empty_new_session_stays_hidden`、`…::cold_start_user_message_stays_listed`、`…::malformed_transcript_rows_still_listed_after_reopen`、`…::busy_empty_session_is_listed_until_cold_start`、`src/client_protocol/ws_wp166_tests.rs::session_list_not_blocked_by_inflight_subscribe`

### C11 set-model（模型切换持久化）
- 守的行为：模型切换与 control-plane 变更行同事务提交，失败一起回滚；仅接受 catalog 可选的引用。
- 守护测试：`src/session/manager.rs::tests::model_change_commits_state_and_control_plane_row_together`、`…::failed_model_change_rolls_back_state_and_event_together`、`tests/settings_api.rs::set_session_model_persists_selectable_catalog_ref`、`src/config/settings_writer.rs::tests::model_switch_rejects_a_reference_the_catalog_does_not_declare`

### C12 serve 绑定与 HTTP/WS 鉴权
- 守的行为：非 loopback 必须 require-auth+token；API 只认 Bearer；WS 拒跨源、放行 localhost（含 ::1）。
- 守护测试：`tests/serve_bind_auth.rs::bind_auth_matrix`、`src/serve/run.rs::tests::loopback_only_rejects_non_loopback_even_with_auth`、`tests/g3_auth.rs::api_rejects_query_token_and_accepts_bearer_only`、`tests/g3_auth.rs::ws_handshake_rejects_cross_origin`、`tests/g3_auth.rs::ws_handshake_allows_localhost_origins_including_ipv6_loopback`、`tests/settings_api.rs::config_set_auth_token_rejected`

### C13 全局配置 DB：迁移/seed/修复
- 守的行为：迁移 v0→current 幂等且只增不删，错误 user_version 失败关闭；seed 种默认 agent 并修复违规行。
- 守护测试：`src/config/global_db/migrate.rs::tests::config_global_db_migration_v0_to_current`、`…::re_running_the_migration_is_a_noop`、`…::config_global_db_wrong_user_version_fails_closed`、`src/config/global_db/seed.rs::tests::seed_plants_default_orchestrator_explore_and_general`、`…::ensure_prunes_rows_that_violate_role_rules`、`tests/global_db_seed.rs::config_global_db_migration_and_seed`、`tests/global_db_seed.rs::legacy_role_rule_rows_are_repaired_on_load`

### C14 Settings 写入：turn 期间阻塞、混合提交回滚、密钥掩码、即时生效
- 守的行为：turn 运行时写设置被拒；文件+DB 混合写失败恢复文件；key 掩码返回；provider key 下一 turn 生效。
- 守护测试：`tests/settings_api.rs::settings_write_blocked_during_turn`、`src/config/settings_writer.rs::tests::commit_mixed_restores_file_when_db_step_fails`、`…::commit_mixed_restores_file_when_validate_fails`、`tests/settings_api.rs::settings_provider_key_write_bumps_revision_and_masks_secret`、`tests/settings_api.rs::settings_provider_key_write_is_effective_next_turn`、`src/config/manager.rs::tests::validate_subagent_cannot_bind_subagent_series`

（附：C15 snapshot 文件回滚 `src/session/snapshot.rs::tests::file_level_restore_only_touches_patch_paths`、`…::warm_then_incremental_edit_restore`、`…::restore_missing_track_ref_is_unavailable`、`tests/issue_032_changed_files_restore.rs::t1_spine_failure_restores_changed_files_for_next_seam` —— 横跨 workspace 域，建议并入核心由 GM 定。）

## 初稿漏掉（补）
- C8：`connection.rs::retain_pending_on_disconnect_does_not_deny`、`abort_all_pending_is_aborted_not_deny`、`matching_accept_resolves_allow`、`unknown_grant_is_buffered_as_stray_then_consumed`、`concurrent_asks_resolve_independently` —— WP-186 断线≠Deny 的唯一单测级守护，初稿只收了 reject。
- C11：`manager.rs::model_change_commits_state_and_control_plane_row_together`、`failed_model_change_rolls_back_state_and_event_together`、`settings_writer.rs::model_switch_rejects_a_reference_the_catalog_does_not_declare` —— set-model 原子性无任何收录。
- C12：`serve/run.rs::loopback_only_rejects_non_loopback_even_with_auth`（bind_auth_matrix 未覆盖 loopback_only 维度）。
- C13：`migrate.rs::config_global_db_migration_v0_to_current`、`re_running_the_migration_is_a_noop`、`config_global_db_wrong_user_version_fails_closed`、`seed.rs::seed_plants_default_orchestrator_explore_and_general`、`ensure_prunes_rows_that_violate_role_rules`。
- C14：`settings_writer.rs::commit_mixed_restores_file_when_db_step_fails`、`commit_mixed_restores_file_when_validate_fails`、`manager.rs(config)::validate_subagent_cannot_bind_subagent_series`。
- C6：`manager.rs::begin_turn_rejects_second_while_running`、`turn_reservation_closes_pre_spawn_idle_window`、`revert_during_running_turn_waits_for_finish_and_holds_exclusive_lease`、`start_turn_fanout_spawn_failure_rolls_back_running_and_turn_guard`、`finish_turn_clears_running_immediately_and_allows_rebegin`。
- C7：`manager.rs::enqueue_remove_and_claim_never_touch_the_log`、`a_failed_revert_keeps_the_queue`。
- C9：`delete.rs::tombstone_survives_reload`、`remove_session_returns_descendants_sorted_root_last`、`connection.rs::wp091_delete_parent_with_running_child_returns_descendant_running`、`wp091_delete_child_tombstone_and_notice`。
- C1：`sqlite/ops.rs::persist_receipt_deleted_rejects_surviving_session_row`。
- C10：`ws_wp166_tests.rs::session_list_not_blocked_by_inflight_subscribe`。
- C15：snapshot.rs 3 条 restore 测试。
（合计补 30 条）

## 初稿误判（剔）
- 集成段整批收录：范围内 112 个集成测试全部列为核心，未经判断。保留上表所列约 37 个，其余约 75 个降为非核心，主要包括：
  - `tests/client_protocol_project.rs` 全部 26 个：逐事件→wire 映射/序列化形状（如 `plan_changed_projects_to_turn_event`），属投影细节，不是链路不变式。
  - `tests/client_protocol_telemetry.rs` 3 个：纯 serde roundtrip。
  - `tests/client_protocol_permission.rs::server_hello_omits_llm_ecosystem`、`permission_request_serializes_request_id`、`ws_permission_bridge_assigns_uuid_request_id`：字段形状。
  - `tests/settings_api.rs` 46 个中约 40 个：custom tool/MCP/engines/LSP/preset CRUD 与 400/404 校验，属配置面边角。
  - `tests/session_turn_entry.rs` 4 个、`tests/session_data_sql_death_list.rs` 1 个：架构/源码扫描（death-list），守的是分层约束而非运行行为 → 归“架构守卫”。
  - `tests/session_list_empty_filter.rs` 中 7 个变体（spilled/missing_blob/empty_blob_refs/older_poked/control_plane_rows/child/in_progress）：与 C10 所选 4 条同一判定逻辑的重复变体。
  - `tests/session_transcript.rs::transcript_items_table_exists_after_session_data_bootstrap`、`tests/session_data_bootstrap.rs` 2 个：建表/开库冒烟。
- src 侧：
  - `config/path.rs` 4 条（canon/strip drive/UNC）：属路径安全域，不属 Session 链路（应移交路径安全段）。
  - `sqlite/session.rs` 52 条中约 40 条（compact_checkpoint_body_is_item_json、revert_preview_is_text_not_json_brace、occupancy_uses_latest_bill…、snip_stale_results_removes_orphan_outputs 等）：序列化/边界细节或与所选测试同一断言的变体。
  - `manager.rs` 中 `turn_status_hook_fires_on_start_and_release`、`g3_turn_finished_reason_non_empty_on_error_termination`：hook/字段非空，非链路。
  - `connection.rs` wp091 中 retry/enqueue/pending_enqueue 3 条：与 `wp091_agent_run_on_child_rejected_readonly` 同一 readonly 守卫的重复入口。
  - `protocol.rs::buffer_state_wire_is_seq_cursor_not_count_window`：形状测试，被 C5 覆盖。

## 回归测（全列）
 - `src/client_protocol/connection.rs` :: `g4_compact_while_running_is_agent_already_running_retryable` — WP-024
 - `src/client_protocol/connection.rs` :: `unknown_grant_is_buffered_as_stray_then_consumed` — WP-091, wp091
 - `src/client_protocol/connection.rs` :: `wp091_agent_cancel_child_writes_parent_notice_no_wake` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_agent_cancel_reports_false_when_idle` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_agent_run_on_child_rejected_readonly` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_buffer_load_returns_subagent_binding_states` — WP-099, wp091, wp099
 - `src/client_protocol/connection.rs` :: `wp091_delete_child_tombstone_and_notice` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_delete_parent_with_running_child_returns_descendant_running` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_delete_running_child_rejected` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_enqueue_on_running_child_rejected_readonly` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_list_sessions_includes_empty_child` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_pending_enqueue_on_child_rejected_readonly` — wp091
 - `src/client_protocol/connection.rs` :: `wp091_retry_on_child_rejected_readonly` — wp091
 - `src/client_protocol/connection.rs` :: `wp099_child_still_readonly_for_user` — wp091, wp099
 - `src/client_protocol/connection.rs` :: `wp099_session_info_last_turn_cancelled_by_user_parent_null` — wp091, wp099
 - `src/client_protocol/connection.rs` :: `wp099_user_cancel_child_notice_by_user_and_no_wake` — wp091, wp099
 - `src/client_protocol/project.rs` :: `auto_compact_failure_maps_to_compaction_failed_not_retryable` — ISSUE-036, wp025, wp186
 - `src/client_protocol/status_track.rs` :: `installed_hook_is_registered` — wp099
 - `src/client_protocol/status_track.rs` :: `last_turn_cancelled_by_survives_reload` — wp099
 - `src/client_protocol/status_track.rs` :: `turn_end_cancelled_by_user_parent_null` — wp099
 - `src/client_protocol/status_track.rs` :: `turn_finished_event_carries_cancelled_by` — wp099
 - `src/client_protocol/ws_wp166_tests.rs` :: `gc_stale_empty_sessions_deferred_until_settled` — WP-166, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `no_projection_frames_stranded_after_fastlane_list` — wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `serve_state_does_not_run_snapshot_maintenance_inline` — WP-166, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `session_list_error_emits_operation_error_and_rpc_error` — WP-166, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `session_list_fastlane_matches_controller_list` — WP-091, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `session_list_not_blocked_by_inflight_subscribe` — WP-091, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `startup_settled_fires_on_first_new_reply_only` — WP-166, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `startup_settled_fires_without_list_or_new` — WP-166, wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `subscribe_replay_frames_unchanged_with_concurrent_list` — wp166
 - `src/client_protocol/ws_wp166_tests.rs` :: `subscribe_unsubscribe_order_preserved_with_list` — wp166
 - `src/config/global_db/seed.rs` :: `migration_fills_missing_primary_code_search_only` — wp149
 - `src/config/global_db/seed.rs` :: `migration_keeps_existing_false_row` — wp149
 - `src/config/global_db/seed.rs` :: `migration_runs_once_on_repeated_startup` — wp149
 - `src/config/global_db/seed.rs` :: `seed_plants_default_orchestrator_explore_and_general` — WP-197
 - `src/config/settings_writer.rs` :: `commit_mixed_restores_file_when_db_step_fails` — ISSUE-017, WP-008
 - `src/config/settings_writer.rs` :: `commit_mixed_restores_file_when_validate_fails` — WP-149
 - `src/config/settings_writer.rs` :: `workspace_custom_without_rules_hides_global_rules` — ISSUE-017, WP-008
 - `src/config/settings_writer.rs` :: `write_agent_existing_agent_switched_to_primary_not_auto_bound` — wp149
 - `src/config/settings_writer.rs` :: `write_agent_new_primary_explicit_false_stays_false` — wp149
 - `src/config/settings_writer.rs` :: `write_agent_new_primary_missing_code_search_binds_true` — wp149
 - `src/config/workspace.rs` :: `load_workspace_state_deferred_does_not_warm` — WP-166
 - `src/config/workspace.rs` :: `load_workspace_state_uses_external_snapshots_dir` — WP-166
 - `src/session/manager.rs` :: `begin_turn_rejects_second_while_running` — WP-024
 - `src/session/manager.rs` :: `cancel_during_starting_turn_cancels_on_start` — wp090
 - `src/session/manager.rs` :: `cancel_leaves_pending_queue_intact` — wp090
 - `src/session/manager.rs` :: `cancel_turn_sync_reports_idle_when_nothing_running` — wp090
 - `src/session/manager.rs` :: `g3_turn_finished_reason_non_empty_on_error_termination` — WP-024
 - `src/session/manager.rs` :: `g4_g5_compact_rejected_while_running_and_turn_restarts_after_error` — WP-024
 - `src/session/manager.rs` :: `s1_reopen_prune_keeps_revision_in_sync` — WP-024
 - `src/session/manager.rs` :: `turn_status_hook_fires_on_start_and_release` — wp090
 - `src/session/snapshot.rs` :: `maintain_reads_active_ids_under_lock_no_false_orphan` — WP-166
 - `src/session/snapshot.rs` :: `maintain_snapshots_waits_for_workspace_lock` — WP-166
 - `src/session/snapshot.rs` :: `snapshot_track_skips_nested_git_dir_entry` — WP-166
 - `src/session/snapshot.rs` :: `track_before_warm_matches_warm_then_track` — WP-166
 - `src/types/death_list_gate.rs` :: `session_seq_g1_envelope_vocab_matches_mental_model` — ISSUE-013
 - `src/types/death_list_gate.rs` :: `session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept` — ISSUE-013
 - `tests/client_protocol_project.rs` :: `turn_finished_auto_compact_failure_is_compaction_failed` — WP-024, WP-025, WP-069
 - `tests/client_protocol_project.rs` :: `turn_finished_marks_a_failed_reconnect_retryable` — WP-024, WP-025, WP-069
 - `tests/issue_032_changed_files_restore.rs` :: `t1_spine_failure_restores_changed_files_for_next_seam` — issue_032
 - `tests/issue_032_changed_files_restore.rs` :: `t2_files_changed_append_failure_soft_restores` — issue_032
 - `tests/issue_032_changed_files_restore.rs` :: `t3_successful_files_changed_does_not_restore` — issue_032
 - `tests/issue_032_changed_files_restore.rs` :: `t4_empty_changed_skips_restore` — issue_032
 - `tests/settings_api.rs` :: `settings_retrieval_refresh_endpoint_accepts_when_cold` — WP-131
- 补充（初稿未标为回归，但对应 WP-186 修复）：`src/client_protocol/connection.rs::tests::retain_pending_on_disconnect_does_not_deny`、`src/client_protocol/connection.rs::tests::abort_all_pending_is_aborted_not_deny`、`src/client_protocol/project.rs::tests::snapshot_always_serializes_pending_permissions_even_when_empty`、`src/client_protocol/project.rs::tests::wire_phase_writes_waiting_for_user_reads_alias`

## 非核心测试（按类别计数，约）
| 类别 | 数量 |
|---|---|
| 序列化/wire 形状与事件投影（client_protocol_project/telemetry、project.rs、protocol.rs、types/*、authority.rs 2） | ~45 |
| 配置面 CRUD/校验边角（settings_api 余 ~40、settings_writer 余 ~19、config/manager 余 ~10、workspace.rs 19、guides/resolved/schema/bridge/log_filter ~26） | ~114 |
| sqlite/compact/surface 细节变体（session.rs 余 ~55、schema.rs 余、surface/estimate/media/media_tokens ~30） | ~90 |
| transcript_file 虚拟路径/渲染 | 15 |
| snapshot 跟踪/gitignore/锁细节（snapshot.rs 余 ~21、snapshot_paths 4、workspace_lock 2） | ~27 |
| manager 其余（plan/child_counts/observer/映射 helper 等） | ~25 |
| 架构/源码扫描守卫（types/death_list_gate 13、session_turn_entry 4、sql_death_list 1） | 18 |
| telemetry/rss/pid helper（telemetry/* 12） | 12 |
| serve/web_dist、CLI 交互答题 helper（cli.rs 4）、version 2、turn_guard 2 | ~11 |
| ws_wp166 启动/列表 fastlane 其余、status_track、controller 计量 | ~25 |
| 冒烟（bootstrap、建表、cli/telemetry roundtrip） | ~6 |

## 缺口/无守护的链路
1. **set-model 与 in-flight turn**：无测试覆盖 turn 运行中切模型（拒绝/排队/作用于下一 turn），亦无 RPC 级端到端（settings_api 那条只到 controller 持久化）。
2. **CLI 入口**：`src/cli` 仅 4 个答题 helper 单测，参数解析/serve 子命令启动无守护。
3. **authority.rs**：仅 2 个 item roundtrip，权威来源判定语义无守护。
4. **version.rs**：仅 channel 非空/dev 冒烟。
5. **telemetry**：只有进程 RSS helper，日志订阅/上报无行为测试（client_protocol_telemetry 仅 serde）。
6. **WS 握手 + 鉴权 + subscribe 冷启动 hydrate 一体化**：g3_auth 只测握手门，未有“鉴权后 subscribe → snapshot 含 pending_permissions”的端到端。
7. 重复守护：`tests/serve_bind_auth.rs::bind_auth_matrix` 与 `src/serve/run.rs::case_a..d` 断言完全相同，可二选一。
