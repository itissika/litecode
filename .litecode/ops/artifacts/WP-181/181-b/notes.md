# WP-187 notes / handoff

## Cold recovery (Q4 — out of scope)

181-a R1: after process restart, oneshot / in-flight tool stack is dead.
This package **does not** implement full “resume then continue tool loop”.

- Hot-path ask contract is fixed in `PhasePermissionSink`.
- If resume shows non-empty `pending_permissions` but phase Idle: prefer session hydrate emitting waiting phase (186 snapshot); runtime only guarantees the next live `ask`.
- Full continue-run → future WP / session+runtime joint package.

## Manager `apply_event_to_progress` (Q5)

Not modified. Enter/leave emit paired `PermissionAwaiting` bools; Manager already flips progress on those. If integration proves PhaseChanged alone leaves bool stale, ping backend_mid before touching session domain.
