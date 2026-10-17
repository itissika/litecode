# WP-191 impl-notes (Runtime)

**Branch:** `runtime/wp-191-request-health`  
**Baseline:** `main` @ `838b6ed`  
**Status:** implemented; **cargo not run** (FE WP-192 holds light slot — await 中层放行)

## Files changed

| File | Change |
|------|--------|
| `src/runtime/observer.rs` | Extend `InternalEvent::LlmRequestBuilt` with health fields (`#[serde(default)]`) |
| `src/client_protocol/protocol.rs` | Mirror fields on `WireEvent::LlmRequestBuilt` |
| `src/client_protocol/project.rs` | Project all new fields Internal → Wire |
| `src/runtime/exec.rs` | Fill health in `build_model_request`; `classify_llm_request_health`; toast → `SnapshotNotice{level:warn}`; emit moved to build (removed from `call_model_complete`); unit tests |
| `src/llm/replay_compat.rs` | `StripCiphertextStats` + read-only `reasoning_wholly_omitted` beside strip; **strip keep/drop unchanged** |
| `src/llm/mod.rs` | Re-export `StripCiphertextStats` |
| `src/client_protocol/controller/mod.rs` | Untouched (`..` already ignores new fields) |

## Strip / compact core

- **`strip_rebased_signatures`:** not modified (behaviour or signature).
- **`strip_foreign_ciphertext`:** behaviour identical (still only clears `encrypted_content`; text/summary stay). Return type is now `StripCiphertextStats` with a second **read-only** counter.
- **`retain_own_reasoning` / compact:** not modified.
- Call sites updated to `.ciphertexts_stripped` where they asserted the old `usize`.

## Severity / code ranking (plan §3.2 / §4)

1. `reasoning_dropped > 0` **or** `reasoning_wholly_omitted > 0` → **toast** / `thinking_not_refilled`
2. `!bind_matches_sticky` → **toast** / `bind_sticky_mismatch`
3. `ciphertexts_stripped > 0` (text retained) → **debug** / `foreign_strip`
4. `rebased_stripped > 0` → **debug** / `rebased_sig_strip`
5. `projection_rewrote` → **debug** / `projection_rewrite` (always `false` this package)
6. else → **silent**

Toast additionally emits `SnapshotNotice { level: "warn", message }` for existing FE toast path.

`reasoning_wholly_omitted`: Messages target counts every foreign ciphertext clear (encode needs signature); other endpoints count only when no text/summary remains on the Item after clear.

## Unit test names

- `request_health_tests::thinking_not_refilled_when_summary_drops_foreign_reasoning`
- `request_health_tests::thinking_not_refilled_when_messages_wholly_omits_foreign_signature`
- `request_health_tests::same_model_continuation_with_own_signature_is_silent`
- `request_health_tests::foreign_strip_with_text_retained_is_debug_not_toast`
- `request_health_tests::bind_sticky_mismatch_is_toast`
- `replay_compat::tests::messages_foreign_strip_counts_as_wholly_omitted`
- `replay_compat::tests::responses_foreign_strip_keeps_text_not_wholly_omitted`

Suggested (when slot open):  
`LC_MIN_MEM_GIB=2 cargo test --lib request_health_tests messages_foreign_strip_counts_as_wholly_omitted responses_foreign_strip_keeps_text_not_wholly_omitted -- --nocapture`

## Gaps

- `projection_rewrote` always `false` — observation gap (see acceptance).
- No full AgentSession integration test (classify + strip fixtures cover §4).
