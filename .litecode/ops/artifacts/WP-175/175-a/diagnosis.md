# WP-178 / 175-a · H1 diagnosis (provider)

**Date:** 2026-10-15  
**Branch surveyed:** `ops/team-framework` (read-only `src/`)  
**Hypothesis H1:** `commandcode/gpt-6-luna` (Responses SSE) — missing `*_tokens_details` on `usage` and/or unknown SSE `type` → `ResponsesCodec::parse_stream_event` fails → `on_broken_stream` **GiveUp** when `produced_items=true` → turn ends as error (not necessarily panic).

---

## Verdict

| Claim | Status |
|--------|--------|
| Luna routes through **Responses SSE** (`endpoint_type = "responses"`) | **Confirmed** (catalog) |
| Unknown SSE `type` (except `keepalive`) fails deserialization | **Confirmed** (code + unit test + replay) |
| `usage` without `input_tokens_details` / `output_tokens_details` fails after harden | **Confirmed** (authority types + replay) |
| `map_max_effort_to_xhigh` does **not** create missing details objects | **Confirmed** (harden tests + code) |
| Mid-stream parse failure with prior deltas → **GiveUp** (no reconnect) | **Confirmed** (code) |
| Failure surfaces as **TurnError**, not codec panic | **Confirmed** (agent/runtime path) |
| **Live Luna** actually emits these malformed frames | **Not observed** (no wire on this VM; see `wire-unavailable.md`) |

**Overall:** H1 is **confirmed as a real, reachable failure mechanism** on the Luna Responses path. It is **not falsified** (we did not prove Luna always sends compliant SSE). Live wire is still needed to pin which of the two sub-mechanisms (unknown type vs sparse usage) matches production, or whether another hypothesis (H2) dominates.

**Recommend:** Open a **separate fix package** (codec harden / unknown-event policy) after optional live wire; do not implement in WP-175.

---

## Stable repro (synthetic, no API key)

### A — Unknown event type after text delta (GiveUp path)

1. Feed SSE in `sse-fixtures/unknown_vendor_event.sse.txt`: text delta → `response.vendor_telemetry` → completed.
2. On the real codec, after the delta, `acc.is_empty()` is false → `parse_stream_event` error → `on_broken_stream(..., produced_items=true)` → **GiveUp** → `preserve_partial` → `LlmStreamInterrupted` if partial sealed.

Evidence replay (mirrors `parse_stream_event` + Luna `usage_patch`):

```text
unknown_type: ERR ResponseStreamEvent: unknown variant `response.vendor_telemetry`, expected one of ...
```

(full log: `h1-verify-output.txt`)

In-tree test (unknown type, no stream):

```bash
CARGO_BUILD_JOBS=1 cargo test --lib 'llm::codec::responses::tests::keepalive_is_skipped_and_other_unknown_types_still_fail' -- --exact
```

### B — `response.completed` with sparse `usage` (after delta)

1. Feed `sse-fixtures/completed_usage_no_details.sse.txt` (delta then completed with only `input_tokens` / `output_tokens`).
2. `responses_harden::fill_token_details` does not add missing detail **objects**; serde requires `InputTokenDetails` / `OutputTokenDetails` on `ResponseUsage`.

Replay:

```text
completed_usage_no_details: ERR ResponseStreamEvent: missing field `input_tokens_details`
```

Control (same shape + details): `completed_usage_with_details` → **OK (parsed event)**.

### C — Falsification attempt (what does *not* fail H1)

- `keepalive` events: skipped (`Ok(None)`), covered by in-tree test above.
- Completed usage **with** empty detail objects: harden fills `cached_tokens` / `reasoning_tokens` inside existing objects (`fill_token_details_only_touches_existing_objects` test).

---

## Root-cause evidence

### Catalog: Luna is Responses + `map_max_effort_to_xhigh`

`src/provider_catalog/default_catalog.toml` (commandcode `gpt-6-luna`):

- `endpoint_type = "responses"`
- `usage_patch = "map_max_effort_to_xhigh"`

Same pattern for `opencode/gpt-6-luna` and `opencode-go/gpt-6-luna` (see `code-map.md`).

Codec selection: `src/llm/codec/mod.rs:31-33` — `EndpointKind::Responses => ResponsesCodec`.

### Authority usage shape (non-optional details)

`async-openai` 0.41.1 `ResponseUsage` (registry excerpt copied to `authority-response_usage.snippet.rs`):

- `input_tokens_details: InputTokenDetails` (required)
- `output_tokens_details: OutputTokenDetails` (required)

### Harden gap (documented by product test)

`src/llm/codec/responses_harden.rs:141-153` — `fill_token_details` only mutates **existing** `input_tokens_details` / `output_tokens_details` objects.

Test `fill_token_details_only_touches_existing_objects` (`responses_harden.rs:227-238`) asserts `output_tokens_details` stays absent when not present in input JSON.

### Unknown SSE types

`ResponseStreamEvent` uses `#[serde(tag = "type")]` with a closed enum (`async-openai` `stream.rs`); no catch-all variant.

Product test expects `unknown variant` for `not-a-real-event` (`responses.rs:1489-1499`).

---

## Failure boundary: TurnError vs panic

| Stage | Behavior |
|--------|----------|
| `parse_stream_event` Err | Returns `LitecodeError::Llm(...)` string — **no panic** |
| Stream loop with `produced_items=true` | `on_broken_stream` → **GiveUp** immediately (`http.rs:217-219`) |
| After GiveUp on parse error | `preserve_partial(error, &acc)` → `LlmStreamInterrupted { partial }` if acc non-empty (`http.rs:294-307`) |
| Agent | `LlmStreamInterrupted` → persist partial pads → `TurnOutcome::Error` (`agent/core.rs:63-83`); other LLM errors → `TurnOutcome::Error` (`:83`) |
| Runtime | `finalize_agent_outcome` emits `TurnEndReason::Error` (`runtime/mod.rs:906-910`) |
| Panic | Only if turn **thread** panics; logged `"agent turn thread panicked"` (`session/manager.rs:3138-3141`) — **not** the normal H1 codec path |

Observed on this VM: **no live turn**; boundary is from static trace + existing tests only.

---

## Codec / contract file:line map

| Topic | Location |
|--------|----------|
| `parse_stream_event` (JSON → harden → deserialize) | `src/llm/codec/responses.rs:165-183` |
| `keepalive` skip | `responses.rs:174-176` |
| Stream read loop; parse Err → `on_broken_stream` | `responses.rs:479-491` (also tail `:527-536`) |
| `on_broken_stream` GiveUp when `produced_items` | `src/llm/codec/http.rs:212-221` |
| `preserve_partial` → `LlmStreamInterrupted` | `http.rs:294-307` |
| `harden` / `MapMaxEffortToXhigh` | `responses_harden.rs:18-28` |
| `fill_token_details` (existing objects only) | `responses_harden.rs:141-153` |
| Unknown type test | `responses.rs:1489-1499` |
| `forward_stream_event` / H2 terminal ids | `stream_contract.rs:634+` (out of scope unless parse succeeds) |
| Wire hook | `wire_dump.rs` (not run — see `wire-unavailable.md`) |

---

## H2 / H3 boundary (diagnostic only)

- **H2** (duplicate item id / projection index) runs **after** successful parse in `forward_stream_event` / `StreamProjection` — not triggered by the fixtures above.
- **H3** (FE ErrorBoundary / thread panic) may **follow** H1 TurnError but is not required to explain codec failure.

---

## Artifacts index

| File | Purpose |
|------|---------|
| `diagnosis.md` | This report |
| `wire-unavailable.md` | Live `LITECODE_LLM_WIRE` note |
| `h1-verify-output.txt` | Replay stdout |
| `h1-verify/` | Minimal parse replay (run outside root workspace: copy to `/tmp` and `cargo run`) |
| `sse-fixtures/*.sse.txt` | Synthetic SSE evidence |
| `authority-response_usage.snippet.rs` | Copy of authority usage struct |
| `cargo-test-snippet.txt` | In-tree harden test run |
| `brief.md`, `plan-draft.md` | WP inputs |
| `../code-map.md`, `../plan.md` | Parent maps |

---

## Done-when (brief)

- [x] Wire or SSE fixture evidence in artifacts  
- [x] H1 conclusion with boundaries  
- [x] Codec path file:line map  
- [x] No `src/` behavior changes  
- [x] Heavy compile: `cargo check` + targeted `cargo test --lib` (2 tests) + small `h1-verify` crate (`CARGO_BUILD_JOBS=1`)

---

## Recommended follow-up (implement elsewhere)

1. **Wire:** One Luna turn with tools + `LITECODE_LLM_WIRE`; diff failing `data:` JSON against fixtures A/B.  
2. **Fix package (if wire matches B):** Create `input_tokens_details` / `output_tokens_details` in harden when `usage` object exists; optionally derive `total_tokens`.  
3. **Fix package (if wire matches A):** Policy for vendor-specific types (skip vs log vs fail).  
4. **Do not merge fix into WP-175** (diagnose-only parent).
