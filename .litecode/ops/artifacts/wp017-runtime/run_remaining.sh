#!/usr/bin/env bash
set -u
ROOT=/workspace/litecode
LOGDIR=$ROOT/.litecode/ops/artifacts/wp017-runtime
SUMMARY=$LOGDIR/summary.tsv

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
  # if multiple "test result" lines (lib + bins), sum passed from all "test result" lines
  if rg -q 'test result:' "$log"; then
    passed=$(rg 'test result:.*([0-9]+) passed' -or '$1' "$log" | awk '{s+=$1} END {print s+0}')
    failed=$(rg 'test result:.*([0-9]+) failed' -or '$1' "$log" | awk '{s+=$1} END {print s+0}')
    ignored=$(rg 'test result:.*([0-9]+) ignored' -or '$1' "$log" | awk '{s+=$1} END {print s+0}')
  fi
  echo -e "${group}\t${filter}\t${ec}\t${passed}\t${failed}\t${ignored}\t${log}" >> "$SUMMARY"
  echo "→ exit=$ec passed=$passed failed=$failed ignored=$ignored"
}

# C retry) agent pending_continue stop_check
run_one lib_agent "stop_check" \
  cargo test --lib -- pending_continue_tests::stop_check

# D) architecture
run_one architecture_layers "runtime_child_sessions" \
  cargo test --test architecture_layers -- runtime_and_protocol_do_not_open_child_sessions

# E) stream_projection settle + fail-closed (module filter = all in module; curated subset documented in artifact)
run_one lib_stream "stream_projection" \
  cargo test --lib -- runtime::stream_projection::tests::

# F) death_list gates
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

# G) llm reconnect + http
run_one lib_llm "reconnect_http" \
  cargo test --lib -- \
  compaction_scope_turns_a_terminal_failure_into_cleared \
  retry_emits_waiting_then_connecting_then_cleared \
  exhausted_budget_emits_failed_without_another_wait \
  non_retryable_status_after_a_retry_does_not_clear \
  send_cancellable_retries_transient_status_before_stream_starts \
  send_cancellable_does_not_retry_explicit_client_failure \
  first_success_emits_no_reconnect_notice

# H) llm_resolve
run_one lib_llm_resolve "compaction_binding" \
  cargo test --lib -- compaction_binding_uses_its_own_provider_and_credentials

# I) settings_writer
run_one lib_config "commit_mixed+core" \
  cargo test --lib -- \
  commit_mixed_restores_file_when_db_step_fails \
  commit_mixed_restores_file_when_validate_fails \
  config::settings_writer::tests::turn_blocks_write \
  settings_writes_hand_agents_a_runnable_model \
  provider_key_write_stores_the_credential_and_masks_it \
  workspace_engines_commit_file_and_event_together

# J) bridge + resolved
run_one lib_bridge "repair_resolved" \
  cargo test --lib -- \
  active_models_require_a_credential \
  agent_config_carries_the_model_ref \
  global_and_workspace_partitions_are_disjoint \
  active_models_follow_the_credential_map

# K) provider_catalog
run_one lib_catalog "provider_catalog_core" \
  cargo test --lib -- \
  embedded_catalog_is_valid_and_complete \
  references_are_provider_slash_model_and_lookup_splits_once \
  request_url_comes_from_endpoint_type \
  reasoning_tiers_inherit_from_provider_and_override_per_model \
  provider_id_format_is_enforced

# L) types
run_one lib_types "fixtures_display" \
  cargo test --lib -- \
  golden_item_fixtures_deserialize \
  compaction_failed_display_matches_empty_summary_path \
  user_text_item_roundtrip \
  assistant_text_item_is_output_assistant_role

# M) global_db_seed
run_one global_db_seed "all" \
  cargo test --test global_db_seed

# N) settings_api main chain
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
