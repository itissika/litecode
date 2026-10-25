# Compaction architecture

This document is the current contract for context compaction. The refactor plan
in `dev/compaction-refactor-plan.md` is historical design input; where they
differ, this document and the source gates win.

## One call mode, one execute path

Compaction is a call mode of the current main request, not an agent. It reuses
the active main model/provider, system prompt, tool schema, request prefix, and
session affinity. There is no independent compaction agent, model binding,
system prompt, prompt pack, or fallback model configuration.

The unique production execution path is:

1. `ContextMeter` measures the current request/projected working set.
2. `compact_transcript` runs the deterministic mechanical prune via
   `apply_mechanical_prune_if_worthwhile`. A prune that gets below the pressure
   line commits stable `PruneReplacement` surface rows and succeeds without an
   LLM summary call.
3. Otherwise `compact_transcript` selects a keep-recent cut, freezes the prefix
   baseline, and enters the summary stage.
4. `first_pass_compaction` runs only the summary attempt loop and invokes
   `call_llm_compact_once_with_timeout` with the main request shape plus
   `COMPACTION_INSTRUCTION`. `MAX_SUMMARY_ATTEMPTS` is the total attempt budget
   (exactly two attempts), not an additional retry count. The retry reuses the
   exact same request (prefix, instruction, and output cap). A summary is
   accepted only when the provider finish is `Completed`, its trimmed text is
   non-empty, it contains no tool call or image, and
   `summary_tokens < source_tokens`. There is no section, quality-score,
   anchor, placeholder, prior-summary, or latest-user review heuristic.
5. A successful checkpoint is committed as one role-free `compacted` row. The
   summary output cap and keep-recent retention come from
   `context_pipeline::limits`; the keep-recent policy is not redefined here.

There is no second-pass, aggressive-truncate, branch, or split-turn rewrite
path. Provider context overflow may force one bounded recovery run, recorded as
`overflow_retry = 1` in lifecycle telemetry.

## Durable authority

`CompactionCheckpoint { summary, from, to }` is the authority object used by
cut, rebase, role, and previous-summary decisions. It has no role. The durable
`CompactedBody` stores only raw `summary`, `from`, and `to`.

`src/session/working.rs` is the only projector that turns a checkpoint into a
model-visible **user** message:

```
{CHECKPOINT_PREAMBLE}

<compacted-summary>{summary}</compacted-summary>
```

The projector never writes this framing back to the log. Legacy summary text
prefixes (`[Conversation summary]`, `[Aggressive summary]`) are stripped only
at this projection boundary for old-data readability; no compaction decision
may inspect them.

## ContextMeter and token accounting

`ContextMeter` is the single server-owned source for occupancy, pressure,
manual eligibility, prompt limit, and UI display. It uses the provider/model
capacities directly: `W = context_window_max` and `O = max_output`. Therefore
`prompt_limit = W - O` and `compact_at = min(0.8W, W - O)`, with non-negative
saturating arithmetic for degenerate windows. There is no safety-headroom
allowance, secondary output clamp, default-tier participation, or independent
`O` budget. The persisted `ContextMode::Standard`/`Max` preference no longer
selects `W` for compaction.

Provider usage is the occupancy bill; the local request envelope, projected
suffix, and media estimate supply the breakdown. Token accounting uses the
shared helpers in `session::estimate`; no compaction module keeps a second
threshold formula or a second breakdown implementation.

Auto compaction triggers only when `used_tokens > compact_at` (strictly
greater). Manual compaction is allowed only when `ContextMeter` says the
inclusive 30%-of-`W` manual ratio gate is met. Auto, manual, prepared, and UI
paths consume the same meter and therefore the same `W`/`O`.

## Lifecycle and telemetry

The unified lifecycle event is `InternalEvent::CompactionLifecycle`. It carries
`trigger`, `stage`, `operation_id`, optional failure kind/error, and
`CompactionTelemetry`:

- `tokens_before`
- `tokens_pruned`
- `tokens_after`
- `summary_attempts`
- `overflow_retry`

The succeeded event is emitted exactly once per execute path. Prune-only
success reports `summary_attempts = 0`. Failed events keep the same
trigger/stage identity and subtype mapping. If the second identical summary
attempt fails acceptance, the compaction is a hard failure: no checkpoint is
written and lifecycle telemetry carries the failure subtype and
`summary_attempts = 2`. A mechanical prune that already committed before the
summary stage remains committed.

## Structural gates

`tests/compaction_structure_gates.rs` enforces the high-risk invariants:

- one compaction execute path; no second-pass/aggressive rewrite path;
- summary-to-user projection only in `src/session/working.rs`;
- one `ContextMeter`/threshold/breakdown source;
- no hidden compaction agent/profile/prompt/config binding;
- no Pi-style `<previous-summary>`, `previous_summary`, branch, or split-turn
  state in the compaction path.
