# WP-208 Provider domain review (baseline `3acfe31`)

Scope: `src/llm/**`, `src/provider_catalog/**`, and integration tests whose primary subject is provider streaming/codec behavior (`tests/agent_e2e_responses.rs`, `tests/turn_prefix_stability.rs`). **221** tests total (lib **207**: llm **172**, provider_catalog **35**; integration **14**).

Method: read test bodies and assertions; test names and the script draft (`core-chains.md`) are not trusted.

---

## 1. Core chains

### SSE line reader (`src/llm/codec/sse.rs`)

Framed SSE bytes are decoded into complete `data:` lines without corrupting UTF-8 across chunk boundaries, with BOM/CRLF handling and hard limits on noise and line size.

- `src/llm/codec/sse.rs::multibyte_char_split_across_chunks_is_preserved`
- `src/llm/codec/sse.rs::leading_bom_is_stripped_once`
- `src/llm/codec/sse.rs::crlf_lines_strip_cr`
- `src/llm/codec/sse.rs::skips_noise`
- `src/llm/codec/sse.rs::trailing_line_without_newline_is_kept_for_finish`
- `src/llm/codec/sse.rs::oversized_line_is_a_hard_error`

### Stream contract — function-call event gate (`stream_contract.rs`)

Out-of-order Responses tool-call events are normalized (synthetic `output_item.added`, name patches, seq ordering) before the accumulator runs.

- `src/llm/codec/stream_contract.rs::delta_before_added_synthesizes_added_first`
- `src/llm/codec/stream_contract.rs::empty_name_then_done_with_name_emits_patch_added`
- `src/llm/codec/stream_contract.rs::real_output_index_zero_is_preserved_not_treated_as_unknown`
- `src/llm/codec/stream_contract.rs::synthesized_seqs_stay_above_all_provider_seqs`
- `src/llm/codec/stream_contract.rs::empty_item_id_delta_does_not_synthesize_added`

### Stream contract — accumulator, terminals, cancel (`stream_contract.rs`)

Live deltas become authority `Item`s; `response.completed` / `response.incomplete` / `response.failed` and user cancel resolve to items, errors, or `Canceled` without duplicating ids.

- `src/llm/codec/stream_contract.rs::incomplete_terminal_yields_items`
- `src/llm/codec/stream_contract.rs::failed_terminal_still_errors`
- `src/llm/codec/stream_contract.rs::text_delta_seal_incomplete_without_terminal`
- `src/llm/codec/stream_contract.rs::resolve_empty_cancel_is_canceled`
- `src/llm/codec/stream_contract.rs::output_text_done_replaces_streamed_text`
- `src/llm/codec/stream_contract.rs::output_item_done_replaces_the_streamed_shell`
- `src/llm/codec/stream_contract.rs::duplicate_item_id_in_completed_output_is_refused`
- `src/llm/codec/stream_contract.rs::real_stream_shape_normalizes_to_one_item_per_id_in_order`

### HTTP send — retry, cancel, empty-stream give-up (`src/llm/codec/http.rs`)

`send_cancellable` honors cancel, retries only retryable pre-stream failures, and surfaces reconnect failure without treating user cancel as failed.

- `src/llm/codec/http.rs::send_cancellable_returns_canceled_when_token_fires`
- `src/llm/codec/http.rs::send_cancellable_retries_transient_status_before_stream_starts`
- `src/llm/codec/http.rs::send_cancellable_does_not_retry_explicit_client_failure`
- `src/llm/codec/http.rs::cancel_while_waiting_does_not_emit_failed`
- `src/llm/codec/http.rs::empty_stream_loss_on_a_spent_budget_emits_failed`
- `src/llm/codec/http.rs::retry_emits_waiting_then_connecting_then_cleared`

### Reconnect UI — compaction turn (`src/llm/reconnect.rs`)

A terminal reconnect `Failed` during compaction is cleared so the UI does not stay stuck in failed state.

- `src/llm/reconnect.rs::compaction_scope_turns_a_terminal_failure_into_cleared`

### Responses codec — streaming product path (`src/llm/codec/responses.rs`)

Responses SSE is forwarded to `on_event`; final `Item`s come from the terminal payload; cancel/disconnect/proxy misconfiguration behave as specified.

- `src/llm/codec/responses.rs::stream_events_are_forwarded_and_items_come_from_the_terminal`
- `src/llm/codec/responses.rs::keepalive_between_events_does_not_abort_the_stream`
- `src/llm/codec/responses.rs::cancel_after_a_delta_seals_incomplete`
- `src/llm/codec/responses.rs::cancel_before_any_event_is_canceled`
- `src/llm/codec/responses.rs::transport_disconnect_returns_partial_items_with_an_explicit_error`
- `src/llm/codec/responses.rs::truncated_sse_after_items_is_terminal_and_keeps_partials`
- `src/llm/codec/responses.rs::non_event_stream_content_type_surfaces_the_proxy_body`

### Responses codec — request encode & reasoning replay (`responses.rs`)

Wire body matches catalog capabilities; foreign/own reasoning ciphertext and summary replay follow quirk rules.

- `src/llm/codec/responses.rs::plain_model_sends_the_authority_shape`
- `src/llm/codec/responses.rs::undeclared_dialect_replays_only_its_own_ciphertext`
- `src/llm/codec/responses.rs::summary_replay_sends_own_id_and_summary_text`
- `src/llm/codec/responses.rs::replay_is_quirk_gated_and_synthesizes_missing_reasoning`

### Responses stream hardening (`src/llm/codec/responses_harden.rs`)

Vendor/tool-call JSON shapes are coerced so downstream parsing sees string arguments and patched usage fields where configured.

- `src/llm/codec/responses_harden.rs::tool_call_object_arguments_become_a_json_string`
- `src/llm/codec/responses_harden.rs::tool_call_string_arguments_stay_verbatim`
- `src/llm/codec/responses_harden.rs::deepseek_patch_also_fills_total_and_effort`

### Messages codec — wire encode & tool pairing (`messages.rs`)

Anthropic-shaped bodies preserve prefix continuity, repair missing tool results, and round-trip thinking signatures inside an open tool turn.

- `src/llm/codec/messages.rs::later_turn_extends_the_previous_messages_prefix`
- `src/llm/codec/messages.rs::a_missing_tool_result_is_repaired`
- `src/llm/codec/messages.rs::thinking_and_tool_signatures_round_trip_inside_the_open_turn`

### Messages codec — streaming (`messages.rs`)

Messages SSE completes with signature preserved; retries/cancel/disconnect/error timing match Chat/Responses policy.

- `src/llm/codec/messages.rs::stream_round_trip_keeps_the_signature_and_sends_the_version_header`
- `src/llm/codec/messages.rs::empty_stream_retries_until_a_terminal_arrives`
- `src/llm/codec/messages.rs::stream_error_before_items_retries_and_after_items_preserves`
- `src/llm/codec/messages.rs::disconnect_after_items_keeps_the_partial`
- `src/llm/codec/messages.rs::cancel_after_a_delta_seals_incomplete`

### Messages SSE → Responses events synth (`messages_synth.rs`)

Anthropic stream events are synthesized into Responses-shaped stream events with stable parallel tool ids and dropped-thinking accounting.

- `src/llm/codec/messages_synth.rs::message_start_records_dropped_thinking_without_changing_the_stream`
- `src/llm/codec/messages_synth.rs::parallel_tool_calls_keep_their_own_ids`
- `src/llm/codec/messages_synth.rs::thinking_text_and_tool_round_trip_with_the_signature`

### Chat completions codec — encode, replay, stream (`chat.rs`)

Chat wire encoding replays reasoning into assistant turns; streaming emits early tool `added`, usage on terminal, and skips whitespace-only assistant shells.

- `src/llm/codec/chat.rs::tools_and_reasoning_replay_into_one_assistant_turn`
- `src/llm/codec/chat.rs::recorded_reasoning_is_replayed_as_reasoning_content`
- `src/llm/codec/chat.rs::stream_tool_call_opens_the_item_before_the_arguments_delta`
- `src/llm/codec/chat.rs::stream_usage_lands_on_the_terminal_response`
- `src/llm/codec/chat.rs::stream_whitespace_only_content_leaves_no_message_item`
- `src/llm/codec/chat.rs::stream_text_only_becomes_an_assistant_message`

### Chat SSE synth (`chat_synth.rs`)

Chat completion deltas that are whitespace-only must not create a message item until visible content arrives.

- `src/llm/codec/chat_synth.rs::whitespace_only_content_never_births_a_message`

### Cross-item replay compatibility (`replay_compat.rs`)

Reasoning ciphertext/signatures are stripped or retained based on producer provider, endpoint kind, model ref, and prefix rewrite — text stays on the item when Responses replay allows.

- `src/llm/replay_compat.rs::ciphertext_goes_back_only_to_its_producer_and_text_always_stays`
- `src/llm/replay_compat.rs::summary_replay_keeps_only_this_providers_reasoning`
- `src/llm/replay_compat.rs::ciphertext_does_not_cross_protocols_on_the_same_provider`
- `src/llm/replay_compat.rs::messages_signature_does_not_survive_a_rewritten_prefix`
- `src/llm/replay_compat.rs::producer_is_the_newest_header_at_or_before_the_item`

### Provider catalog — load, resolve, strict validation (`provider_catalog/tests.rs`)

Embedded seed is the sole catalog source; models resolve to URLs by endpoint kind; invalid TOML is rejected, not trimmed.

- `src/provider_catalog/tests.rs::embedded_catalog_is_valid_and_complete`
- `src/provider_catalog/tests.rs::shared_catalog_is_the_embedded_seed_and_does_not_read_a_file`
- `src/provider_catalog/tests.rs::references_are_provider_slash_model_and_lookup_splits_once`
- `src/provider_catalog/tests.rs::request_url_comes_from_endpoint_type`
- `src/provider_catalog/tests.rs::unknown_field_is_refused`
- `src/provider_catalog/tests.rs::codec_capability_mismatch_is_refused_not_trimmed`
- `src/provider_catalog/tests.rs::summary_replay_requires_responses_and_rejects_the_text_quirk`

### Agent e2e — Responses codec on product runtime (`tests/agent_e2e_responses.rs`)

Real runtime turn against local Responses SSE: text-only, tool loop, incomplete seal to transcript.

- `tests/agent_e2e_responses.rs::agent_e2e_responses_text_only`
- `tests/agent_e2e_responses.rs::agent_e2e_responses_tool_loop_hits_max_steps`
- `tests/agent_e2e_responses.rs::agent_e2e_incomplete_seals_text_to_disk`

### Turn prefix stability — prompt-cache identity (`tests/turn_prefix_stability.rs`)

Within a turn (and across reload with reused provider ids), already-sent `ModelRequest` JSON prefix must not be rewritten by persist/stream/seal.

- `tests/turn_prefix_stability.rs::tool_loop_llm_request_prefix_json_is_stable`
- `tests/turn_prefix_stability.rs::prepare_after_stream_persist_seal_keeps_prior_prefix`
- `tests/turn_prefix_stability.rs::persist_item_reused_function_call_id_must_not_rewrite_final_row`
- `tests/turn_prefix_stability.rs::persist_item_reused_message_id_must_not_rewrite_final_row`
- `tests/turn_prefix_stability.rs::next_turn_reload_must_keep_sent_row_when_provider_reuses_id`

---

## 2. Draft corrections (vs script `core-chains.md` llm block)

### Add (draft missed or under-specified)

| Test / chain | Reason |
|---|---|
| `stream_contract.rs::{delta_before_added_synthesizes_added_first, empty_name_then_done_with_name_emits_patch_added, real_output_index_zero_is_preserved_not_treated_as_unknown, synthesized_seqs_stay_above_all_provider_seqs, empty_item_id_delta_does_not_synthesize_added}` | Draft listed only accumulator/terminal tests; gate normalization is a separate core path. |
| `stream_contract.rs::duplicate_item_id_in_completed_output_is_refused` | Guards transcript duplication; only named in code comment, not in draft. |
| `stream_contract.rs::{output_text_done_replaces_streamed_text, output_item_done_replaces_the_streamed_shell}` | Draft had `output_*_done` partially; both done kinds are asserted separately. |
| `responses.rs::{transport_disconnect_returns_partial_items_with_an_explicit_error, truncated_sse_after_items_is_terminal_and_keeps_partials}` | Stream interruption after partial items; not in draft. |
| `messages.rs::{stream_round_trip_keeps_the_signature_and_sends_the_version_header, disconnect_after_items_keeps_the_partial, overloaded_status_is_retried}` | Messages end-to-end stream + headers; draft only listed retry/cancel/error subset. |
| `messages.rs::{later_turn_extends_the_previous_messages_prefix, a_missing_tool_result_is_repaired, thinking_and_tool_signatures_round_trip_inside_the_open_turn}` | Core encode path for Messages tool turns. |
| `messages_synth.rs::thinking_text_and_tool_round_trip_with_the_signature` | Full synth round-trip, not just dropped-thinking metadata. |
| `chat.rs::{tools_and_reasoning_replay_into_one_assistant_turn, recorded_reasoning_is_replayed_as_reasoning_content, stream_text_only_becomes_an_assistant_message}` | Chat encode + stream happy path; draft omitted replay and text-only stream. |
| `chat_synth.rs::whitespace_only_content_never_births_a_message` | Chat dialect synth invariant; draft had only codec whitespace stream test. |
| `replay_compat.rs` (five tests listed above) | Entire module absent from draft; required for multi-provider reasoning safety. |
| `responses.rs::{plain_model_sends_the_authority_shape, undeclared_dialect_replays_only_its_own_ciphertext, summary_replay_sends_own_id_and_summary_text, replay_is_quirk_gated_and_synthesizes_missing_reasoning}` | Request/replay encode chains absent from draft. |
| `responses_harden.rs::deepseek_patch_also_fills_total_and_effort` | Usage/effort patch path; draft only had tool argument coercion. |
| `http.rs::retry_emits_waiting_then_connecting_then_cleared` | Reconnect notice sequence; draft listed cancel/empty-stream but not successful retry UX. |
| `reconnect.rs::compaction_scope_turns_a_terminal_failure_into_cleared` | Compaction-specific reconnect clearing. |
| `sse.rs::{trailing_line_without_newline_is_kept_for_finish, oversized_line_is_a_hard_error, skips_noise}` | Draft had 3/7 SSE tests; these assert distinct failure/boundary behavior. |
| All `provider_catalog/tests.rs` core guards (seven above) | Draft folded catalog into llm block only via codec headers; no catalog validation chain. |
| `tests/agent_e2e_responses.rs` (three tokio tests) | Product-path integration; draft buried all of `tests/` in one 440-test blob. |
| `tests/turn_prefix_stability.rs` (five tests above) | Provider prompt-cache contract; not in draft (draft cited non-existent `code_mode_tool_list_prefix_stays_stable_across_the_turn` on this baseline). |

### Remove (draft misjudged as core)

| Test | Reason |
|---|---|
| `src/llm/codec/wire_dump.rs::messages_stream_folds_deltas_and_adds_cache_into_input` | Wire debug/dump folding for humans; not product streaming semantics. |
| `src/llm/codec/wire_dump.rs::stream_fold_counts_both_dialects_and_the_terminal` | Same — observability ledger, not authority item path. |
| `src/llm/codec/chat.rs::encodes_system_and_user_messages_with_stream_usage` | Subsumed by stream usage + encode tests; asserts static JSON shape only. |
| `src/llm/codec/http.rs::send_cancellable_retries_transient_status_before_stream_starts` | Keep one retry test (`retry_emits_waiting…`); this duplicates retry behavior without reconnect assertions. |
| `src/llm/codec/messages_synth.rs::message_delta_replaces_the_dropped_thinking_list` | Second-order synth detail; `message_start_records…` + round-trip cover the invariant. |

---

## 3. Regression tests (in scope)

No in-scope test **names** or bodies cite `WP-*`, `ISSUE-*`, or commit hashes. In-scope **bug-pin** tests identified from comments/assertions:

| Test | Pin |
|---|---|
| `src/llm/codec/chat.rs::recorded_reasoning_is_replayed_as_reasoning_content` | Comment: regression guard — model-owned reasoning must replay as `reasoning_content` regardless of item id. |
| `src/llm/codec/stream_contract.rs::duplicate_item_id_in_completed_output_is_refused` | Comment: duplicate terminal ids caused two transcript rows; must error. |

(`src/llm/codec/http.rs` production comment mentions WP-188; there is no WP-tagged test in scope.)

---

## 4. Non-core (by category)

Everything not listed in §1 is non-core by default. **~86** tests in §1 → **~135** non-core (221 − 86).

| Category | Count (approx.) | Notes |
|---|---:|---|
| Responses request encoding matrix (thinking tiers, temperature, json, tools, media, headers, usage patch, parse quirks) | 28 | `responses.rs` unit encode/parse tests |
| Messages transcript folding (reminders, fences, cache, media degrade, signature retention across turns/compaction) | 22 | `messages.rs` non-stream unit tests |
| Messages synth edge (delta replace, max_tokens/refusal, redacted, citations ignored) | 5 | `messages_synth.rs` |
| Chat encode/header/usage/idempotence edge cases | 12 | `chat.rs` minus core six |
| Chat synth seam/heuristic reporting | 5 | `chat_synth.rs` minus one core |
| Chat/Messages usage merge helpers | 4 | `chat_usage` / `messages_usage` / `responses` usage_patch |
| `replay.rs` assistant-segment synthesis | 5 | Used by chat encode; covered indirectly by replay core tests |
| `replay_compat.rs` legacy/stats edge | 6 | Legacy origin + wholly-omitted counters |
| `responses_harden.rs` shell/token fill | 3 | Beyond tool-args + deepseek patch |
| HTTP transport misc (retry delay math, connection pool, first_success, non_retryable clear, exhausted budget duplicate) | 6 | `http.rs` |
| SSE `extracts_data_payload` | 1 | Redundant with `skips_noise` for core |
| Wire dump / session GC / token null stats | 9 | `wire_dump.rs` |
| Provider catalog seed spot checks & validation edge (tiers, gateways, bailian, bounds, headers, enums sync) | 28 | `provider_catalog/tests.rs` minus seven core |
| Integration fixture smoke | 1 | `agent_e2e_responses.rs::responses_fixtures_are_not_chat_dialect` |
| Turn prefix — session/reminder/compact paths not about LLM prefix bytes | 5 | e.g. `persist_roundtrip_preserves_item_json`, `hanging_call_pad…`, `restore_row_lands…`, `state_reminders…`, `job_exit_mid_turn…` |

---

## Summary

| Metric | Value |
|---|---|
| Tests in scope | 221 |
| Core guard tests (§1, unique) | 86 |
| Non-core (§4) | ~135 |
| In-scope WP/ISSUE-named regressions | 0 |
| In-scope comment-pinned regressions | 2 |

Coordinator can save this file as `.litecode/ops/artifacts/WP-208/review-provider.md`.