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
   (two total attempts: the first call plus one correction retry), not an
   additional retry count. Each `SummaryArtifact` is reviewed for finish,
   structure, quality, and savings before it is accepted.
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
manual eligibility, and safety headroom. Provider usage is the occupancy bill;
the local request envelope, projected suffix, and media estimate supply the
breakdown. Token accounting uses the shared helpers in
`session::estimate`; no compaction module keeps a second threshold formula or
a second breakdown implementation.

Manual compaction is allowed only when `ContextMeter` says the inclusive
manual ratio gate is met. Auto pressure uses the same meter, not a duplicate
budget check.

## Lifecycle and telemetry

The unified lifecycle event is `InternalEvent::CompactionLifecycle`. It carries
`trigger`, `stage`, `operation_id`, optional failure kind/error, and
`CompactionTelemetry`:

- `tokens_before`
- `tokens_pruned`
- `tokens_after`
- `summary_attempts`
- `review_score`
- `overflow_retry`

The succeeded event is emitted exactly once per execute path. Prune-only
success reports `summary_attempts = 0` and no review score. Failed events keep
the same trigger/stage identity and subtype mapping.

## Structural gates

`tests/compaction_structure_gates.rs` enforces the high-risk invariants:

- one compaction execute path; no second-pass/aggressive rewrite path;
- summary-to-user projection only in `src/session/working.rs`;
- one `ContextMeter`/threshold/breakdown source;
- no hidden compaction agent/profile/prompt/config binding;
- no Pi-style `<previous-summary>`, `previous_summary`, branch, or split-turn
  state in the compaction path.
