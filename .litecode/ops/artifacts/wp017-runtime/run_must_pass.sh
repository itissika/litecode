#!/usr/bin/env bash
set -u
ROOT=/workspace/litecode
LOGDIR=$ROOT/.litecode/ops/artifacts/wp017-runtime
SUMMARY=$LOGDIR/summary.tsv
: > "$SUMMARY"
echo -e "group\tfilter\texit\tpassed\tfailed\tignored\tlog" >> "$SUMMARY"

run_one() {
  local group="$1"
  local filter="$2"
  shift 2
  local safe
  safe=$(echo "$group-$filter" | tr '/ :' '___' | tr -cd 'A-Za-z0-9._-')
  local log=$LOGDIR/${safe}.log
  local start
  start=$(date '+%Y-%m-%dT%H:%M:%S%z')
  echo "=== [$start] $group :: $*  (filter=$filter) ===" | tee "$log"
  set +e
  ( cd "$ROOT" && "$@" ) >>"$log" 2>&1
  local ec=$?
  set -e
  local passed failed ignored
  passed=$(rg -o '[0-9]+ passed' "$log" | tail -1 | rg -o '[0-9]+' || echo 0)
  failed=$(rg -o '[0-9]+ failed' "$log" | tail -1 | rg -o '[0-9]+' || echo 0)
  ignored=$(rg -o '[0-9]+ ignored' "$log" | tail -1 | rg -o '[0-9]+' || echo 0)
  echo -e "${group}\t${filter}\t${ec}\t${passed}\t${failed}\t${ignored}\t${log}" >> "$SUMMARY"
  echo "→ exit=$ec passed=$passed failed=$failed ignored=$ignored"
}

# A) agent_loop critical turn + WP-014 caliber 1
run_one agent_loop "turn+compact_fail" \
  cargo test --test agent_loop -- \
  single_text_response_stops_loop \
  function_call_then_text_grows_transcript \
  max_steps_with_perpetual_tools \
  cancelled_breaks_loop \
  persist_failure_rolls_back_working_set \
  auto_compact_fail_stops_turn_without_model_call

# B) WP-014 calibers 2+3 (+ helper auto path)
run_one compact_failure_calibers "all" \
  cargo test --test compact_failure_calibers

# C) agent core stop_check
run_one lib_agent "stop_check" \
  cargo test --lib -- agent::core::tests::stop_check

# D) architecture layer gate (runtime-owned)
run_one architecture_layers "runtime_child_sessions" \
  cargo test --test architecture_layers -- runtime_and_protocol_do_not_open_child_sessions

# E) stream_projection settle + fail-closed
run_one lib_stream "settle_fail_closed" \
  cargo test --lib -- runtime::stream_projection::tests:: \
  a_streamed_item_settles_into_the_row_it_opened \
  an_item_the_call_never_confirmed_settles_incomplete \
  a_returned_list_that_omits_a_streamed_item_fails_the_call \
  same_call_identity_conflicts_fail_closed \
  terminal_duplicate_id_fails_closed

# F) death_list gates owned by runtime/types/model-config
run_one lib_death "runtime_owned_gates" \
  cargo test --lib -- \
  stream_projection_uses_only_the_lifecycle_writes \
  death_list_dialect_tokens_absent_from_src \
  death_list_provider_catalog_is_the_only_llm_source \
  death_list_codecs_are_provider_agnostic \
  death_list_db_runtime_store_only_uses_provider_credentials \
  death_list_no_vendor_model_catalog_fetch \
  compact_llm_path_does_not_bypass_unified_provider \
  session_seq_g1_envelope_vocab_matches_mental_model \
  session_seq_g4_wire_speaks_seq \
  session_seq_g6_derived_paths_use_surface_not_pointers \
  session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept

# G) llm reconnect + http retry/exhausted
run_one lib_llm "reconnect_http" \
  cargo test --lib -- \
  llm::reconnect::tests::compaction_scope_turns_a_terminal_failure_into_cleared \
  llm::codec::http::tests::retry_emits_waiting_then_connecting_then_cleared \
  llm::codec::http::tests::exhausted_budget_emits_failed_without_another_wait \
  llm::codec::http::tests::non_retryable_status_after_a_retry_does_not_clear \
  llm::codec::http::tests::send_cancellable_retries_transient_status_before_stream_starts \
  llm::codec::http::tests::send_cancellable_does_not_retry_explicit_client_failure \
  llm::codec::http::tests::first_success_emits_no_reconnect_notice

# H) llm_resolve compaction binding
run_one lib_llm_resolve "compaction_binding" \
  cargo test --lib -- runtime::llm_resolve::tests::compaction_binding_uses_its_own_provider_and_credentials

# I) settings_writer commit_mixed harden + core writes
run_one lib_config "commit_mixed+core" \
  cargo test --lib -- \
  config::settings_writer::tests::commit_mixed_restores_file_when_db_step_fails \
  config::settings_writer::tests::commit_mixed_restores_file_when_validate_fails \
  config::settings_writer::tests::turn_blocks_write \
  config::settings_writer::tests::settings_writes_hand_agents_a_runnable_model \
  config::settings_writer::tests::provider_key_write_stores_the_credential_and_masks_it \
  config::settings_writer::tests::workspace_engines_commit_file_and_event_together

# J) bridge repair + resolved partitions
run_one lib_bridge "repair_resolved" \
  cargo test --lib -- \
  config::bridge::tests::active_models_require_a_credential \
  config::bridge::tests::agent_config_carries_the_model_ref \
  config::resolved::tests::global_and_workspace_partitions_are_disjoint \
  config::resolved::tests::active_models_follow_the_credential_map

# K) provider_catalog core
run_one lib_catalog "provider_catalog_core" \
  cargo test --lib -- \
  provider_catalog::tests::embedded_catalog_is_valid_and_complete \
  provider_catalog::tests::references_are_provider_slash_model_and_lookup_splits_once \
  provider_catalog::tests::request_url_comes_from_endpoint_type \
  provider_catalog::tests::reasoning_tiers_inherit_from_provider_and_override_per_model \
  provider_catalog::tests::provider_id_format_is_enforced

# L) types fixtures + CompactionFailed display (WP-007)
run_one lib_types "fixtures_display" \
  cargo test --lib -- \
  types::golden_item_fixtures::tests::golden_item_fixtures_deserialize \
  types::error::tests::compaction_failed_display_matches_empty_summary_path \
  types::transcript::tests::user_text_item_roundtrip \
  types::transcript::tests::assistant_text_item_is_output_assistant_role

# M) global_db_seed integration (boot repair path post WP-006)
run_one global_db_seed "all" \
  cargo test --test global_db_seed

# N) settings_api subset (main-chain settings write gate — not every CRUD)
run_one settings_api "main_chain" \
  cargo test --test settings_api -- \
  settings_write_blocked_during_turn \
  settings_summary_masks_api_key \
  settings_provider_key_write_bumps_revision_and_masks_secret \
  settings_put_orphan_model_ref_returns_400 \
  settings_auth_endpoint_removed

echo
echo "======== SUMMARY ========"
column -t -s $'\t' "$SUMMARY" || cat "$SUMMARY"
