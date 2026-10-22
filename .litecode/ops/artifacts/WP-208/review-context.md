# WP-208 核对 · 上下文域（review-context）

- 基线：`3acfe31`（ops: WP-207 deliver）。工作树 HEAD=`28d3c85`，与基线在 src/tests 下无差异；内容一律按 `git show 3acfe31:<path>` 读取。
- 只读：没跑 cargo，没改 src/tests，没 commit，没 checkout。只新增本文件。
- 范围：src/context_pipeline、src/engines（session_search、code_search、code_search_ipc、path_index 等）、src/knowledge、src/reminder（含 #[cfg(test)]）；tests/{stage_a_ctx_consistency, compact_failure_calibers, manual_compact_e2e, code_search_ipc, optional_code_search, code_index_tick_death_list, working_set_identity}。另外把范围外的 `src/tools/session_search.rs` 也纳入，因为 WP-205 的回归测在那里。
- 判断方式：读测试体的断言和被测函数。初稿 core-chains.md 是按文件整批列出的（比如 stage_a 25 条、optional_code_search 17 条全列），所以不可信。

## 1. 核心链路 → 守护测试

| # | 链路（守的行为） | 守护测试 | 为什么能守住 |
|---|---|---|---|
| C1 | 装配 / view：内存工作集必须和 DB fold 一致；pending 尾巴没有 log_seq | `tests/working_set_identity.rs::append_rows_reader_working_set_matches_full_fold`、`::pipeline_begin_turn_pending_tail_has_no_log_seq`、`::commit_applied_window_matches_fold` | 逐行比较 reader 和 full fold 的结果；断言 pending 行 seq=None |
| C2 | view 补齐未答 call：只在 LLM 视图里补，不落盘 | `tests/stage_a_ctx_consistency.rs::unanswered_calls_pad_llm_view_only_not_disk`、`src/context_pipeline/mod.rs::tests::padding_alignment_uses_order_not_provider_ids` | 断言请求里有 padding、DB 里没有；对齐按顺序而不是 provider id |
| C3 | budget：优先用 provider bill，前缀之外的后缀才估算；compact 后 bill 作废 | `src/context_pipeline/budget.rs::tests::provider_bill_covers_its_prefix_and_estimates_only_the_suffix`、`::rewritten_view_does_not_reuse_a_provider_bill`、`::compact_retires_the_in_memory_bill_and_the_stored_one` | 断言 token 数的来源；内存和存储里的 bill 都被清掉 |
| C4 | 触发 auto compact：阈值按真实用量判断，低 bill 不会因为本地估算偏高而误触发 | `src/context_pipeline/compact.rs::tests::low_provider_bill_does_not_compact_a_hot_local_estimate`、`::provider_bill_over_threshold_compacts_then_the_bill_stays_retired`、`tests/stage_a_ctx_consistency.rs::real_usage_feeds_compaction_decision_input` | 断言 compact 是否被调用，以及调用后 bill 的状态 |
| C5 | manual compact 门槛：30% 含边界；manual 绕过 auto 阈值 | `src/context_pipeline/budget.rs::tests::manual_compact_gate_is_inclusive_at_thirty_percent`、`tests/stage_a_ctx_consistency.rs::manual_compact_bypasses_auto_threshold_and_preserves_full_history` | 断言边界值；断言全史保留 |
| C6 | keep_recent 切点：不拆开 tool 对和 assistant 段，切完放得进 history budget；超过硬上限仍强制处理 | `src/context_pipeline/keep_recent.rs::tests::tool_pair_not_split`、`::assistant_segment_not_split_between_reasoning_and_message`、`::compact_split_fits_under_history_budget`、`tests/stage_a_ctx_consistency.rs::keep_recent_skip_still_enforces_hard_limit_when_over_budget` | 断言切点索引落在段边界上；断言 token 数 ≤ budget |
| C7 | compact 只吃已持久化的前缀，保留未提交尾巴；重建时 call_id 不重复 | `tests/stage_a_ctx_consistency.rs::compact_eats_only_persisted_prefix_and_keeps_uncommitted_tail`、`::compact_rebuild_does_not_repeat_kept_call_ids`、`tests/working_set_identity.rs::keep_recent_compact_reader_matches_fold_checkpoint_then_kept` | 断言尾巴的条目还在；断言 call_id 唯一；断言 checkpoint+kept 等于 fold |
| C8 | compact 请求：超时、取消、关 thinking、用调用方给的 cap；只送纯文本（剥掉 media 和 reasoning） | `src/context_pipeline/compact.rs::tests::compact_request_times_out_on_silent_provider`、`::compact_request_observes_cancel_while_waiting`、`::compact_request_disables_thinking_and_uses_caller_cap`、`src/context_pipeline/keep_recent.rs::tests::serialize_strips_media_before_the_compact_prompt` | 断言错误类型、请求参数，以及 prompt 里没有 media |
| C9 | compact 失败回滚：transcript/工作集恢复到快照，lease 释放后可重试，失败后 turn 停止且不再调模型 | `tests/compact_failure_calibers.rs::compact_failure_restores_transcript_snapshot`、`::auto_compact_failure_restores_pipeline_working_set`、`::manual_compact_failure_releases_lease_for_retry`、`::reconnect_then_auto_compact_failure_stops_turn`、`tests/manual_compact_e2e.rs::manual_compact_retry_after_failure_completes` | 断言 after.len()==before_len、!is_compacting、!is_session_busy、CompactionFailed 及 fail_kind |
| C10 | compact 已持久化但 reload 失败：以 DB 为准，报 Succeeded | `src/context_pipeline/compact.rs::tests::compact_persist_ok_reload_err_keeps_db_truth` | 注入 reload 错误后断言 Ok(true)、事件为 Succeeded、DB 有 summary |
| C11 | summary / rebased：summary 是 assistant 消息；比 summary 早的 kept 行要 rebase；compact 后续写能接上 | `src/context_pipeline/summary.rs::tests::compact_summary_is_assistant_message_not_user`、`::kept_rows_older_than_the_summary_are_rebased`、`src/context_pipeline/keep_recent.rs::tests::update_prompt_when_leading_summary`、`tests/stage_a_ctx_consistency.rs::compact_then_new_step_persists_and_resumes_full_content` | 断言 role、rebase 标记、增量 prompt 的形态、恢复后内容完整 |
| C12 | media_budget：工具 media 被模型看过一次就丢；用户 media 留到本 turn 结束；被剥掉的 batch 只 rebase 它的应答 | `src/context_pipeline/media_budget.rs::tests::tool_media_is_dropped_once_the_model_has_seen_it`、`::latest_tool_batch_keeps_every_media_part`、`::user_media_from_an_earlier_turn_is_dropped`、`::only_the_answer_to_a_stripped_batch_is_rebased`、`::summary_strip_removes_every_media_part` | 断言 media part 的个数和 rebase 的范围 |
| C13 | reminder 注入：compact 后 restore 只触发一次；reminder 挂在 checkpoint 上，不额外插 user detail；env 快照变化才再次发出 | `src/reminder/engine.rs::tests::restore_fires_once_after_compaction`、`tests/stage_a_ctx_consistency.rs::compact_reminder_rides_on_checkpoint_not_extra_user_detail`、`::compact_keeps_pending_tail_after_post_compact_reminder`、`src/reminder/sources/tests.rs::tasks_restore_only_after_compaction_and_once`、`::env_emits_once_until_snapshot_changes` | 断言发出次数和 DB 行数 |
| C14 | mentions reminder：user 行后面跟一条隐藏 reminder，revert 时两条一起删；symbol 没变就不重复附 | `src/reminder/mentions.rs::tests::user_row_is_followed_by_a_hidden_mentions_reminder_and_revert_drops_both`、`::unchanged_symbol_is_not_attached_again_until_the_body_changes` | 断言行序和 revert 后的行数 |
| C15 | system/env 装配：AGENTS.md 和 CLAUDE.md 的拼接顺序；hidden agent 不拼 md | `src/context_pipeline/system.rs::tests::primary_splices_agents_md_then_claude_md`、`::hidden_never_splices_md`、`src/context_pipeline/env.rs::tests::build_context_copies_process_snapshot_and_ignores_disk` | 断言 prompt 文本的顺序，以及 md 不出现 |
| C16 | session_search 索引：stale 索引在查询前被追上；索引坏了要报错而不是返回空；增量结果等于重建 | `src/engines/session_search/mod.rs::tests::a_stale_index_is_caught_up_behind_the_query`、`::a_broken_index_is_an_error_not_an_empty_result`、`src/engines/session_search/dense_parity.rs::tests::an_appended_dense_index_equals_a_clean_rebuild`、`src/engines/session_search/semantic_index.rs::tests::wp135_index_error_is_failed_index_failed_and_clears_on_success` | 断言命中结果、Err 变体、两份索引逐项相等 |
| C17 | session_search 检索/呈现：能命中种子文本，scope 和 exclude 生效；超出预算的部分溢出到命名文件；hit 行与虚拟 read 对齐 | `src/engines/session_search/mod.rs::tests::session_text_search_finds_seeded_transcript`、`::context_window_exclude_drops_live_seqs_keeps_archive`、`::agent_view_spills_the_remainder_into_a_named_file`、`src/tools/session_search.rs::tests::search_hit_line_matches_virtual_read`、`src/tools/session_search.rs::tests::compacted_history_below_kept_from_seq_remains_searchable` | 断言命中的 seq、溢出文件名可解析且存在、read 的 data.lines 覆盖 hit 范围并含 token |
| C18 | code_search 索引持久化：多文件原子写，崩溃后可恢复；commit 和 stamps 之间崩溃不会漏 | `src/engines/code_search/atomic.rs::tests::crash_between_renames_leaves_prefix_and_sweep_clears_tmp`、`src/engines/code_search/mod.rs::tests::crash_between_compaction_renames_keeps_chunks_and_vectors_one_to_one`、`src/engines/code_search/reconcile.rs::tests::crash_between_index_commit_and_stamps_rereads_never_misses` | 注入崩溃点后断言 chunks 和 vectors 一一对应、会重读 |
| C19 | code_search IPC：worker 全生命周期；fs 通知后能搜到新文件；meta 不兼容才重建，其余情况走增量 | `tests/code_search_ipc.rs::worker_ping_initialize_warmup_search_shutdown`、`::notify_fs_changes_then_search_hits_new_file`、`::refresh_incremental_then_rebuild_when_meta_incompatible`、`::refresh_stays_incremental_when_all_files_dirty` | 断言每一步的响应、命中、job 类型 |
| C20 | code_search 可选降级：engine 关掉或 worker 崩溃时降级不卡；warming 期间工具已列出；停止时不留僵尸 | `tests/optional_code_search.rs::worker_crash_demotes_engine`、`::engine_desired_off_stops_engine`、`::catalog_lists_tool_while_warming_and_after_warm`、`::stop_during_warmup_leaves_no_zombie_worker`、`::warmup_loads_compatible_index_without_full_rebuild` | 断言 engine 状态、工具列表、进程是否存活 |
| C21 | code_index tick：tick 只做同步，不 build 也不 cool；手动 refresh 不合并 plan | `tests/code_index_tick_death_list.rs::code_index_tick_never_builds_or_cools`、`::manual_refresh_does_not_merge_plans_for_execution`、`tests/optional_code_search.rs::code_index_tick_syncs_without_refresh`、`src/engines/code_search/mod.rs::tests::pending_blocks_cool_drop` | 断言 plan 的种类和 pending 阻止 cool |
| C22 | knowledge 加载：增量结果等于从零重建；turn 只读快照；状态分类 | `src/knowledge/index_tests.rs::incremental_equals_from_scratch_over_random_edits`、`::the_turn_reads_the_snapshot_only`、`::every_file_is_parsed_once_per_stamp`、`src/knowledge/spec_tests.rs::resolution_follows_the_spec`、`src/knowledge/status.rs::tests::classes_cover_errors_warnings_and_enabled` | 随机编辑下逐项相等；断言解析计数；按 spec 用例表比对 |
| C23 | revert 与上下文一致：revert 后提交会丢弃未提交 delta，不重放也不补齐 | `tests/stage_a_ctx_consistency.rs::revert_contract_three_states`、`::commit_after_revert_discards_uncommitted_delta`、`tests/working_set_identity.rs::revert_then_commit_discard_returns_fold_window_and_does_not_append` | 断言三态和 DB 行数不增 |

合计：23 条链路，89 条守护测试（不同测试名去重计）。

## 2. 补（初稿漏掉的核心守护）

1. 整个 `media_budget.rs`（C12 的 5 条）：初稿完全没有 media_budget，而 WP-189 的焦点就是 show-once。
2. `summary.rs::kept_rows_older_than_the_summary_are_rebased`：rebased 不变式唯一的直接守护。
3. `src/tools/session_search.rs::search_hit_line_matches_virtual_read`：WP-205 回归测，初稿的 WP-205 小节漏了它，反而列了 `rejects_empty_query`。
4. `src/reminder/engine.rs::restore_fires_once_after_compaction` 和 `sources/tests.rs::tasks_restore_only_after_compaction_and_once`、`env_emits_once_until_snapshot_changes`：初稿不覆盖 src/reminder。
5. `src/reminder/mentions.rs::user_row_is_followed_by_a_hidden_mentions_reminder_and_revert_drops_both`：守 revert 与隐藏 reminder 成对删除。
6. `src/knowledge/index_tests.rs::incremental_equals_from_scratch_over_random_edits`、`the_turn_reads_the_snapshot_only`：初稿的 knowledge 只出现在回归列表里，没有加载链路。
7. `system.rs::primary_splices_agents_md_then_claude_md`、`hidden_never_splices_md`、`env.rs::build_context_copies_process_snapshot_and_ignores_disk`：上下文装配的入口。
8. `mod.rs::tests::padding_alignment_uses_order_not_provider_ids`：padding 对齐。
9. `session_search/mod.rs::a_broken_index_is_an_error_not_an_empty_result`、`context_window_exclude_drops_live_seqs_keeps_archive`：防止"坏索引返回空结果"，防止当前窗口和历史检索重复。
10. `keep_recent.rs::tool_pair_not_split`、`update_prompt_when_leading_summary`。

## 3. 剔（初稿误判为核心，或守错了链路）

1. `tests/stage_a_ctx_consistency.rs` 整文件 25 条全列。下面这些降为非核心：`llm_completed_emitted_and_last_request_usage_in_turn_stats`、`turn_completed_fires_after_db_commit`、`commit_user_message_returns_session_preview`、`resumed_session_cursor_initialized_from_max_seq`、`orphan_function_call_output_purged_from_db_in_commit_transaction`、`commit_snips_memory_orphan_without_deleting_log_seq`、`idle_revert_truncates_log_for_next_begin_turn`、`agent_persist_after_revert_does_not_replay_or_pad`、`compact_then_send_message_matches_run_with_turn`、`compact_then_send_message_with_shadowed_tool_rows`、`compact_then_agent_run_persists_assistant`、`keep_recent_skip_under_hard_limit_returns_ok_without_compact`、`compact_decision_tracks_real_provider_usage`、`pipeline_compact_reloads_working_set_from_fold`。理由：它们属于 session/协议事件域，或者与 C4/C7/C11/C23 的守护重复覆盖。
2. `tests/optional_code_search.rs` 17 条全列。下面这些剔掉：`hash_embedder_used_in_tests_without_model_download`（测试基建）、`workspace_init_creates_meta_without_vectors`、`warmup_creates_vectors_usearch`、`small_repo_index_round_trip_and_query`、`embedder_id_recorded_in_meta_and_triggers_rebuild`、`pipeline_version_change_triggers_rebuild`、`per_workspace_indexes_are_isolated`、`unbind_does_not_stop_engine`、`build_tool_list_includes_code_search_after_warmup`、`engines_json_enables_code_search_for_workspace`、`code_search_lifecycle_rebuild_stale_consume_stop`。理由：要么是配置/格式类，要么被 C19/C20 覆盖。
3. `tests/code_search_ipc.rs` 11 条全列。剔掉 `protocol_search_hit_roundtrip_serde`（serde 往返，属于格式类），以及 `warmup_and_refresh_follow_disk_excludes`、`refresh_honors_gitignore_switch_from_disk`、`warmup_*_reconciles_without_digest`、`warmup_without_session_db_then_inject_session_lane`、`refresh_updates_when_dirty_is_mostly_deletes`（边角情况，或与 C19 重复）。
4. `compact.rs::compact_request_returns_summary_on_provider_success`、`compact_summary_drops_reasoning_items`、`compact_summary_empty_when_only_reasoning`：happy path 和纯函数边角，已被 C8 的 strip 测试覆盖。
5. `keep_recent.rs::compact_output_tokens_caps_above_product_limit`、`within_keep_or_empty_returns_none`：常量上限和空输入边角。
6. `code_search/atomic.rs::append_syncs_parent_dir_only_when_creating_the_file`、`index_status.rs::job_and_queue_writes_are_atomic`、`semantic_engine.rs::borrow_lexical_wire_works_from_semantic_engine`：分别是 fsync 细节、格式、接线，不在主链路上。
7. `session_search/semantic_index.rs::wp144_bytes_per_batch_independent_of_index_size`：性能/规模类，改列入回归表。`golden.rs::golden_long_row_tiles_losslessly_within_budget`、`trimmer.rs::a_result_under_the_budget_is_left_alone`、`sparse.rs::the_human_outranks_a_shorter_echo_that_repeats_the_phrase`：排序和裁剪的边角，非核心。
8. 初稿 WP-205 小节里的 `src/tools/session_search.rs::rejects_empty_query`、`semantic_unavailable_is_explicit_and_leads_with_sync_pct`：与 WP-205 无关，标错了归属。

## 4. 回归测（全列）

| 测试 | 对应问题 |
|---|---|
| `src/engines/session_search/mod.rs::tests::agent_view_spills_the_remainder_into_a_named_file` | **WP-205**：footer 文案改了以后，溢出文件名的解析要跟上（`named_spill_location`）。 |
| `src/tools/session_search.rs::tests::search_hit_line_matches_virtual_read` | **WP-205**：hit 行号要和虚拟 transcript 的 read（start_line+limit，data.lines）对齐。 |
| `src/tools/session_search.rs::tests::a_result_over_budget_names_the_file_that_holds_the_rest` | WP-205 时一并改动（共用 helper），属于协同回归，不在那两条之内。 |
| `src/context_pipeline/compact.rs::tests::compact_persist_ok_reload_err_keeps_db_truth` | ISSUE-035 / WP-039：持久化之后 reload 失败，必须以 DB 为准。 |
| `src/engines/session_search/semantic_index.rs::tests::wp135_incremental_scans_then_embeds_from_kept_chunks`、`::wp135_index_error_is_failed_index_failed_and_clears_on_success` | WP-135：增量 embed 和索引失败状态。 |
| `src/engines/session_search/semantic_index.rs::tests::wp144_bytes_per_batch_independent_of_index_size` | WP-144：写放大。 |
| `src/engines/session_search/mod.rs::tests::gate_drops_weak_semantic` / `gate_keeps_strong_semantic` | WP-057：宁可不返回，也不要假命中。 |
| `src/engines/code_search/index_status.rs`（含 "WP-057"/"WP-131 cleanup" 断言的测试） | WP-057：dirty 不触发 rebuild；WP-131：旧字段保持删除。 |
| `src/engines/code_search/atomic.rs` 崩溃系列、`reconcile.rs::crash_between_index_commit_and_stamps_rereads_never_misses` | WP-143 / WP-141。 |
| `tests/code_search_ipc.rs::refresh_stays_incremental_when_all_files_dirty` | ISSUE-052 / WP-057。 |
| `tests/manual_compact_e2e.rs::manual_compact_failure_e2e_releases_lease_and_retry_starts`、`::manual_compact_retry_after_failure_completes` | WP-025。 |
| `tests/optional_code_search.rs::catalog_lists_tool_while_warming_and_after_warm` | WP-148 |
| `tests/optional_code_search.rs::code_index_tick_syncs_without_refresh` | WP-079 |
| `tests/optional_code_search.rs::code_search_lifecycle_rebuild_stale_consume_stop`、`::pipeline_version_change_triggers_rebuild`、`::warmup_loads_compatible_index_without_full_rebuild` | WP-143 |
| `tests/stage_a_ctx_consistency.rs::manual_compact_bypasses_auto_threshold_and_preserves_full_history`、`::pipeline_compact_reloads_working_set_from_fold` | ISSUE-013 |
| `src/knowledge/spec_tests.rs::directory_texts_match_the_status`、`::lines_on_a_symbol_are_invalid_lines_in_the_snapshot`、`::oversized_and_binary_files_are_unreadable_everywhere` | WP-050 |

## 5. 其余测试（默认非核心）

计数方法：`git show 3acfe31:<file> | rg -c '#\[(tokio::)?test'`，逐文件统计后求和。范围内总数 **568** = src 501（context_pipeline 61 / engines 370 / knowledge 38 / reminder 32）+ tests 67（stage_a 25 / compact_failure 5 / manual_compact_e2e 2 / code_search_ipc 11 / optional_code_search 17 / tick_death_list 2 / working_set_identity 5）。另外范围外的 src/tools/session_search.rs 纳入了 2 条。

其中核心守护 87 条（范围内）。其余约 481 条，按类别粗分（下面的数字是估计）：
- 纯函数和边角（session_search 的 query_plan/cjk/chunk/slots/relevance/echo、code_search 的 enclosing/code_tokenize/lexical、keep_recent 的 serialize/transcript 等）：约 190
- 格式/解析/文案（agent_view 文案、age 显示、knowledge view/mentions 预览、reminder migrate、protocol serde、system pack marker）：约 110
- 排序/融合/打分（ranking、sparse、fusion、gate）：约 50
- 一致性重复覆盖（parity/golden、stage_a 的会话域部分、IPC 和 optional 的冗余部分）：约 80
- 状态/进度/配置（progress、status_view、engines/mod.rs 34 条、path_index、index_status）：约 45
- 性能/规模：约 6
