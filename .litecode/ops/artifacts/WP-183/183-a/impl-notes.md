# WP-188（183-a）impl notes — Obs-1 + Obs-2 observe-only

**branch:** `runtime/wp-188-obs`  
**no cargo / no commit** in this pass.

## Recovery (2026-10-16 HKT)

Session stash churn / shared worktree branch flips wiped Obs call-site edits.
`/tmp/wp188-full/` initially held **pre-hook** copies of call sites (only
`diagnostics.rs` was complete). Hooks were **re-applied from plan-draft.md**
against live files, then re-backed up to `/tmp/wp188-full/`.

Mid-flight the worktree briefly switched to `ops/wp-186-session-pending-permissions`;
Obs sources were copied to `/tmp/wp188-full/`, branch forced back to
`runtime/wp-188-obs`, and files restored from that backup. Session WIP was
parked in git stash (do **not** pop onto this branch).

## Files created

| Path | Role |
|------|------|
| `src/runtime/diagnostics.rs` | paths, last-turn, tool-audit JSONL, wire-note, DiagContext TLS, panic hook, unit tests |
| `.litecode/ops/artifacts/WP-183/runbook-luna-grep.md` | Luna+grep runbook |
| `.litecode/ops/artifacts/WP-183/183-a/failure-retention.md` | KEEP_* docs; no FAIL_ONLY |
| `.litecode/ops/artifacts/WP-183/183-a/impl-notes.md` | this file |
| `.litecode/ops/artifacts/WP-183/183-a/plan-draft.md` | fill plan (approved) |

## Files modified (current tree)

| Path | Change |
|------|--------|
| `src/runtime/mod.rs` | `pub(crate) mod diagnostics`; `DiagContext::enter` + `note_wire_pointer` in `run_with_turn`; Error path `log_turn_error_wire` + `heartbeat_last_turn` in `emit_turn_completed` |
| `src/runtime/phase.rs` | `emit_phase` → `DiagContext::update_step` + `heartbeat_last_turn` (**not** PhasePermissionSink / AskOutcome) |
| `src/runtime/exec.rs` | beside `InternalEvent::Error(LlmHttp)` → `log_turn_error_wire` |
| `src/cli/cli.rs` | after `setup_logging` → `install_panic_hook(logs_dir.parent())` |
| `src/tool/executor.rs` | `run_tool` audit: start / auth_ask / auth_done / exec_done / panic / cancel (+ resource-busy) |
| `src/tool/pipeline.rs` | join `Err` → audit `phase=panic`; `join_tool_handle` takes `&self` + `invocations` |
| `src/llm/codec/http.rs` | one-line `warn` before GiveUp when `produced_items` (no control-flow change) |

## Explicitly untouched

- `PhasePermissionSink` / `AskOutcome` / authorize branch semantics (`authorize.rs` not modified)
- H1 codec parse logic / retention constants
- session writer / seal / FE / web / client_protocol pending-permission files

## session_id → audit

`ToolPipeline` holds `session_id` (`bind_session`); passes `&session_id` into `run_tool`.
Audit events use that parameter. `turn_id` / `step` come best-effort from
`DiagContext` TLS set in `run_with_turn` (may be absent inside `tokio::spawn` workers).

## Compile-risk guesses (no cargo this pass)

- `pub(crate) mod diagnostics` required so `cli` can call `install_panic_hook`
- `phase.rs` uses private `self.sessions` (OK: child module of `runtime`)
- `AuthResult` matched by ref for `auth_outcome` then by value — should compile
- `ToolSignalLevel` via `crate::types::ToolSignalLevel` for exec_done `ok`
- diagnostics unit tests need `tempfile` (already in Cargo.toml)
- `std::backtrace::Backtrace::force_capture` needs Rust nightly-or-stable-with-feature; crate already may enable `RUST_BACKTRACE` / nightly feature — verify when cargo slot opens

## Deferred

- cargo test `diagnostics::` when backend_mid grants heavy slot (`LC_MIN_MEM_GIB=2`)
- optional: propagate turn_id into spawned tool tasks (TLS does not cross `tokio::spawn`)
- `LITECODE_LLM_WIRE_FAIL_ONLY` — not in this package
- acceptance.md after manual/cargo verify

## Backup

All touched `src/` files mirrored under `/tmp/wp188-full/src_*` (overwrite on each recovery).
