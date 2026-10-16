# WP-187（WP-181-b）impl notes

**branch:** `runtime/wp-187-phase-permission-sink` @ base `4590e84`  
**status:** src changed; **cargo not run** (await backend_mid slot)  
**scope:** `src/runtime/phase.rs` only — no bridge / connection / H1 / FE / manager

## What changed

### `PhasePermissionSink::ask` (`src/runtime/phase.rs`)

**Enter** (before `inner.ask`):

1. `PermissionAwaiting { awaiting: true }`
2. `PhaseChanged { phase: AwaitingPermission { tool, rule_id, summary }, step }`  
   — payloads taken from `AskPrompt`; step from wrapped `AtomicU64`

**Leave** (after `inner.ask`), via `leave_phase`:

1. `PermissionAwaiting { awaiting: false }`
2. By `AskOutcome`:
   - `Allow { .. }` → `PhaseChanged(ExecutingTools)` (unchanged from pre-fix Allow-only path)
   - `Deny` → `PhaseChanged(ExecutingTools)` **(new; Q1)** — leave waiting so phase does not stick
   - `Aborted` → `PhaseChanged(Cancelling)` **(new; Q2)** — no TurnCompleted from sink

Helpers: `emit_phase`, `leave_phase` on `PhasePermissionSink`.

### Not touched (per plan / Q5)

- `observer.rs` — enum shape unchanged (Q3)
- `runtime/mod.rs` — wrap order unchanged (`Cancelling` inner → `Phase` outer)
- `session/manager.rs` — rely on paired `PermissionAwaiting` bools (Q5)
- `permission_bridge.rs` / `connection.rs` / H1 / web — forbidden

## Tests added (`phase::tests`, same file)

| ID | fn | asserts |
|----|-----|---------|
| T1 | `t1_enter_emits_awaiting_bool_then_phase` | enter order: bool true → PhaseChanged(AwaitingPermission{…}); inner called |
| T2 | `t2_allow_leave_executing_tools` | leave bool false + ExecutingTools |
| T3 | `t3_deny_leave_executing_tools` | leave bool false + ExecutingTools; no post-leave AwaitingPermission |
| T4 | `t4_aborted_leave_cancelling` | leave bool false + Cancelling |
| T5 | `t5_cancelling_wrapper_aborted_leave_cancelling` | Phase outer + Cancelling inner + cancelled → Aborted + T4 leave; Recording inner not called |
| — | `reply_outcome_forwarded_unchanged` | Allow{always:true} forwarded |

## Cargo when slot granted

```bash
LC_MIN_MEM_GIB=2 LC_KILL_MEM_GIB=1 CARGO_BUILD_JOBS=1 \
  cargo test -p litecode --lib phase:: -- --nocapture
```

Optional regression (if mid allows): `permission::sinks` cancelling test (unchanged).

## File:line map (post-change)

| region | approx |
|--------|--------|
| struct + wrap | ~16–35 |
| `emit_phase` / `leave_phase` | ~37–55 |
| `ask` enter/leave | ~58–78 |
| `AgentRuntime` helpers | ~80–98 |
| `#[cfg(test)]` T1–T5 | ~100–345 |

Exact lines: see `impl.diff` and `/tmp/wp187-full/phase.rs`.

## Backups

- `/tmp/wp187-full/phase.rs.before` — pre-change
- `/tmp/wp187-full/phase.rs` — post-change copy
- `.litecode/ops/artifacts/WP-181/181-b/impl.diff` — `git diff -- src/runtime/phase.rs`
