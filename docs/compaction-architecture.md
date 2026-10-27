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
   `apply_mechanical_prune_if_worthwhile`. A prune commits only when all three
   hold: every replacement starts at or after the fixed protected line (the
   first 60% of the effective context window, so the cached prefix survives; a
   row that straddles the line stays protected), the batch clears an absolute
   savings floor (`12_288` tokens), and it removes at least 60% of the suffix
   it actually rewrites (from the first replacement to the end of the persisted
   prefix). The projected surface must also fall to or below `compact_at`. A
   committed prune succeeds without an LLM summary call. A prune that misses a
   gate, or still leaves pressure, writes nothing; the summary stage then
   performs the single prefix rewrite. A worthwhile prune always wins over the
   frozen branch summary. If `compact_at` is below the protected line, the eligible
   tail is empty and mechanical prune never fires.
3. Otherwise `compact_transcript` selects a keep-recent cut, freezes the prefix
   baseline, and enters the summary stage. The independent branch builds the
   same plan at the 0.8 prepare line and uses the exact same summary request.
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

There is no second-pass, aggressive-truncate, or split-turn rewrite path.
Provider context overflow may force one bounded recovery run, recorded as
`overflow_retry = 1` in lifecycle telemetry.

## Independent compaction branch

`LITECODE_COMPACT_PREFETCH` selects the auto-compaction timing:
`on` (default), `off` (synchronous compaction), or `shadow` (reserved preview
mode; it follows the synchronous path today). The flag never changes the
durable authority or the request shape.

When `on` and the meter crosses `compact_at`, the trigger boundary first runs
one deterministic prune decision. If the prune alone relieves pressure it
commits and no summary runs. Otherwise the surface is untouched and one frozen
compaction plan is built from those exact rows. The plan is the same plan the
old foreground compaction would have used at that boundary: same model,
instructions, tools, prefix, cut, `kept_from_seq`, and output cap.

The summary and its commit run in one independent branch. The main turn keeps
running below the hard send line and does not wait for it. Every retry uses the
same frozen request/prefix/cut; a failure never re-plans the content. The branch
writes the same single `SessionMutation::Compact` as the foreground path and
reloads the fold. Rows written after the snapshot re-anchor at
`kept_from_seq`, so rows after the cut stay verbatim. The branch emits
`Started` once after freezing its plan and before the summary call, then
`Succeeded` at commit, `Failed` on error, or `Abandoned` when a frozen plan is
rejected without a rewrite. These are observation events only; they do not
gate the branch or the main loop.

The commit has no second 0.8 gate and no second prune decision. If the frozen
plan no longer matches the surface (leading checkpoint changed or
`kept_from_seq` disappeared), or if it had no keep-recent cut (`NoCut`), the
branch writes nothing: below the hard send line the main turn continues, and
at the hard send line one synchronous path may choose a fresh plan with the
same two-attempt summary budget. A summary failure is not silently replaced
by a fresh plan; at the hard line it fails the turn.

Normal turn end does not cancel or join the branch; it finishes and commits on
its own. User cancellation still cancels it. A branch still in flight is kept
across turns and consumed at the next boundary; one that finished before a new
turn is settled at turn start, and its committed checkpoint is loaded by the
new turn.

The hard send line is `min(selected_context_window, prompt_limit - margin)`,
where `margin = max(4096, 2% * context_window)`. It follows the product mode:
standard stops at 100% of its selected window, while max is capped by the
physical prompt ceiling. `compact_at` (0.8) is the prepare line, not the send
line: while the next prompt fits below the hard line, an in-flight summary does
not block the loop. At the hard line the loop waits for the in-flight summary;
with none it runs the synchronous path.

The total summary budget for one plan is two provider attempts
(`MAX_SUMMARY_ATTEMPTS`). The independent branch uses both attempts on the
same frozen request. A normal synchronous compaction (flag off, no branch)
uses the same two-attempt loop.

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
manual eligibility, prompt limit, and UI display. It separates the selected
product window from the provider's physical capacity:

- `context_window` is the selected product window. `standard` uses the model's
  standard/default budget (normally 256K, or the model maximum when smaller);
  `max` uses the provider model maximum.
- `prompt_limit` is the physical prompt ceiling:
  `model_max_context - max_output`.
- `compact_at` is `min(0.8 * selected_context_window, prompt_limit)`, with
  non-negative saturating arithmetic for degenerate windows.
- The hard send line is
  `min(selected_context_window, prompt_limit - margin)`, so standard mode
  stops at its own window (the UI gauge reaches 100%) and max mode stops at the
  physical prompt ceiling minus margin.

There is no secondary output clamp, independent `O` budget, or implicit
substitution of the provider maximum for the selected product window. The
selected `W` is also the `context_window` published to the UI, so the standard
and max modes produce different pressure points.

Provider usage is the occupancy bill; the local request envelope, projected
suffix, and media estimate supply the breakdown. Token accounting uses the
shared helpers in `session::estimate`; no compaction module keeps a second
threshold formula or a second breakdown implementation.

Auto compaction triggers only when `used_tokens > compact_at` (strictly
greater). `compact_at` is the prepare line; `must_compact_before_send(margin)`
is the hard line at `min(selected_context_window, prompt_limit - margin)`.
Auto, manual, branch, and UI
paths consume the same meter and therefore the same `W`/`O`. Manual
compaction is allowed only when `ContextMeter` says the inclusive 30%-of-`W`
manual ratio gate is met.

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
success reports `summary_attempts = 0`. A branch emits Started and Succeeded
when its own commit writes; preparing a summary emits no lifecycle event.
Failed events keep the same trigger/stage identity and subtype mapping. If the
second identical summary
attempt fails acceptance, the compaction is a hard failure: no checkpoint is
written and lifecycle telemetry carries the failure subtype and
`summary_attempts = 2`. A non-relieving prune is never committed, so a failed
summary cannot leave a partial surface rewrite behind.

## Structural gates

`tests/compaction_structure_gates.rs` enforces the high-risk invariants:

- one compaction execute path; no second-pass/aggressive rewrite path;
- summary-to-user projection only in `src/session/working.rs`;
- one `ContextMeter`/threshold/breakdown source;
- no hidden compaction agent/profile/prompt/config binding;
- no Pi-style `<previous-summary>`, `previous_summary`, branch, or split-turn
  state in the compaction path.
