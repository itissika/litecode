# WP-187（WP-181-b）acceptance

**branch:** `runtime/wp-187-phase-permission-sink` @ `4590e84` + dirty `phase.rs`  
**cargo:** **NOT RUN YET** — awaiting backend_mid heavy slot (`LC_MIN_MEM_GIB=2`, `CARGO_BUILD_JOBS=1`)

## done_when checklist

- [x] Enter: `PermissionAwaiting{true}` + `PhaseChanged(AwaitingPermission{tool,rule_id,summary})` implemented in `PhasePermissionSink`
- [x] Leave Allow → ExecutingTools; Deny → ExecutingTools; Aborted → Cancelling; bool false on leave
- [x] No second ask channel; AskOutcome semantics unchanged
- [x] No H1 codec / permission_bridge / connection reject_all / FE / manager edits
- [ ] T1–T5 green under `cargo test -p litecode --lib phase::` (pending slot)
- [ ] deliver → backend_mid review

## Command to run when slot granted

```bash
LC_MIN_MEM_GIB=2 LC_KILL_MEM_GIB=1 CARGO_BUILD_JOBS=1 \
  cargo test -p litecode --lib phase:: -- --nocapture
```

Paste stdout / pass count here after run.

## Evidence paths

- `plan.md` (from plan-draft; Q1–Q6 defaults)
- `impl-notes.md`
- `impl.diff`
- `notes.md` (cold-recovery handoff)
