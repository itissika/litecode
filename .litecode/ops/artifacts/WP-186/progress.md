# WP-186 progress — Session pending_permissions / waiting_for_user

**branch:** `ops/wp-186-session-pending-permissions`  
**time:** 2026-10-16 08:05 CST (Asia/Hong_Kong)  
**cargo:** GREEN — delivering (NEED_RELEASE_SLOT=yes)

## SNAPSHOT_READY: **yes**

Wire/snapshot/attach path is implemented for FE stage B:

1. `SessionSnapshot.pending_permissions: Vec<PermissionRequest>` — `#[serde(default)]` only; **no** `skip_serializing_if` (empty `[]` always present).
2. `WireTurnPhase::AwaitingPermission` serializes as `waiting_for_user`; reads alias `awaiting_permission`.
3. `buffer_snapshot` initializes `pending_permissions: []`.
4. `Projection::snapshot` + `subscribe_checked` fill from `SessionManager::pending_permissions_snapshot` via `pending_records_to_wire` (kind copied explicitly).
5. `awaiting_permission` on turn/live derived from `!pending_permissions.is_empty()`.
6. Unit tests in `project.rs` (`wp186_pending_permissions_tests`) lock always-present empty field + wire rename + frozen shape keys.

## call_id prep: **done** (awaiting NEED_SLOT → now compiling)

1. Removed junk trailer `=== UNTRACKED ===` from `src/session/pending.rs`.
2. `SessionManager::latest_unanswered_call_id_for_tool` — latest unanswered FunctionCall matching tool name from transcript.
3. `connection.rs` upsert links `call_id` via that helper (no longer `None`).
4. `sqlite/pending.rs::backfill_call_id` — fills oldest null-call_id sidecar row by tool name.
5. `seal_in_progress_items`: permission-class FunctionCalls → Final **without** Incomplete + backfill; non-permission still Incomplete. `seal_tail` skip + R4 name-fallback unchanged. SNAPSHOT_READY / always-present `pending_permissions` untouched.

## NEED_RELEASE_SLOT: **yes**

Compile/test results (LC_MIN_MEM_GIB=2 LC_KILL_MEM_GIB=1 CARGO_BUILD_JOBS=1, cargo_locked.sh):

| Step | Result |
|------|--------|
| `check --locked --lib` | EXIT=0 (~16s), warnings only |
| `wp186_pending_permissions_tests` | 4/4 ok |
| `pending_ask_tests` | 6/6 ok (incl. abort≠Deny, retain disconnect) |
| `seal_in_progress_items_marks_incomplete_and_final` | ok |
| death_list filter (lib) | 0 matched (lives in tests/ harness; not re-run full) |

Delivering `--as session --id WP-186`. No commit/push. Did not touch web/board.

## Also implemented earlier

| Area | Status |
|------|--------|
| P1 sidecar table `session_pending_permissions` | CRUD in `sqlite/pending.rs`; ensured from schema |
| `SavePendingPermissions` mutation + SessionData helpers | done |
| Manager in-memory + resume hydrate + upsert/remove/clear | done |
| Connection: upsert on ask; remove on grant; Cancel→Aborted+clear; Quit/disconnect→retain (no Deny) | done |
| `permission_bridge` no-WS → Aborted (R5, no fake Skip) | done |
| `seal_tail_unanswered_calls` skip permission-class (sidecar / ask_user name fallback R4) | done |
| call_id link + seal_orphaned permission posture | **done this pass** |

## Remaining work

1. ~~Compile/test + deliver~~ (this pass).
2. Cold-restore **continue-run** after user reply → **WP-187**.
3. Do **not** commit/push; do **not** touch `web/` / `runtime/mod.rs` / board.

## Files touched (Session territory)

- `src/client_protocol/protocol.rs`
- `src/client_protocol/project.rs`
- `src/client_protocol/controller/mod.rs`
- `src/client_protocol/connection.rs`
- `src/client_protocol/permission_bridge.rs`
- `src/session/pending.rs` **(new)**
- `src/session/mod.rs`
- `src/session/manager.rs`
- `src/session/data/command.rs`
- `src/session/data/mod.rs`
- `src/session/data/writer.rs`
- `src/session/data/sqlite/mod.rs`
- `src/session/data/sqlite/schema.rs`
- `src/session/data/sqlite/read.rs`
- `src/session/data/sqlite/session.rs`
- `src/session/data/sqlite/pending.rs` **(new)**
- `.litecode/ops/artifacts/WP-186/{progress.md,impl.diff,plan.md}`

## Artifacts

- `impl.diff` — tracked diff + untracked new file bodies
- `progress.md` — this file
