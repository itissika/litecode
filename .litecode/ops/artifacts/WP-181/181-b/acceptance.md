# WP-187（181-b）acceptance — PhasePermissionSink

**branch:** `runtime/wp-187-phase-permission-sink` @ `9cd19a6` (+ this evidence commit)  
**baseline:** `4590e84` (WP-186)  
**when:** 2026-10-16 Asia/Hong_Kong

## cargo

```bash
LC_MIN_MEM_GIB=2 LC_KILL_MEM_GIB=1 CARGO_BUILD_JOBS=1 \
  cargo test -p litecode --lib phase:: -- --nocapture
```

**Result: 6 passed; 0 failed**

- t1_enter_emits_awaiting_bool_then_phase
- t2_allow_leave_executing_tools
- t3_deny_leave_executing_tools
- t4_aborted_leave_cancelling
- t5_cancelling_wrapper_aborted_leave_cancelling
- reply_outcome_forwarded_unchanged

Log: `cargo-phase.txt` (compile ~1m21s incremental; pre-existing warnings only)

## Scope

- Only `src/runtime/phase.rs` (+ 181-b artifacts)
- No permission_bridge / connection reject_all / H1 / FE / manager
- Enter: PermissionAwaiting{true} + PhaseChanged(AwaitingPermission{…})
- Leave: Allow/Deny → ExecutingTools; Aborted → Cancelling

## Artifacts

plan.md, impl-notes.md, impl.diff, notes.md (cold-recovery handoff), this file
