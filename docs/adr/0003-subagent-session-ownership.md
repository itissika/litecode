# 0003 — Subagent tools use the same session primitives as humans

Status: accepted

## Context

A child session is a full session. The subagent tool series is agent-side UX
for the same runtime a human uses via `session/start` / `session/stop-turn`.
`SubagentHub` must not own turn lifecycle (create, reserve, start, finish,
join, delete). `SessionManager` must not absorb the hub (job registry, fanout-
specialized exits). Explanation language for the agent need not match the
human UI; the **source of facts** must.

A previous attempt inverted this: it added `start_session_turn` /
`ChildTurnRegistry` / `SubagentJobBoard` on `SessionManager` and spawned
before reserving. That is rejected. A later attempt kept a per-parent
concurrency cap (4) on the hub. That is also rejected: the cap is not a
product rule, and maintaining it is not worth the surface.

## Decision

1. **Same primitives.** Human turns (controller, bash idle) and agent tools
   (`subagent_launch`, `subagent_send`) call `reserve_turn` → `spawn_turn` →
   `SessionManager::start_turn`, in that order. Stop is `cancel_turn_sync`.
   Launch opens a child with the existing `open_child_session`; a failed start
   uses `remove_session`. No `start_session_turn` / `spawn_child_turn` /
   `start_reserved_turn` on the session layer.
2. **Hub is a client.** It lives on `RuntimeHandle` like `TerminalHub`: running
   list, mailbox, waiters, wire snapshot. It subscribes to the child session
   event stream (the same `subscribe` a UI uses) and records the job outcome
   from live `TurnCompleted` (`reason` + `final_text`). If that event is
   missed (`Lagged` / `Closed` / idle without completion), it hydrates the
   last durable `turn/end` and last assistant text. No durable row settles as
   `unknown`, never a synthesized failure. It does not join the turn thread or
   call `finish_turn`. Rendering (`ok` / `stopped` / "max steps reached") is
   agent-facing presentation of those facts, aligned with `turn_error`.
3. **Depth lock is product.** Subagent tools bind only on primary turns
   (`depth == 0` / `SUBAGENT_MAX_DEPTH = 1`). Children cannot nest. There is
   no per-parent concurrency cap.
4. **Session parity.** Child turns use `TurnOptions::agent(...)`; human/idle
   turns use `TurnOptions::default()`. Both go through `spawn_turn`. Busy send
   is the session (`is_turn_running_blocking` / `AgentAlreadyRunning`), not a
   hub latch.

## Consequences

- `tests/session_turn_entry.rs` and `tests/architecture_layers.rs` pin that
  `src/session/` has no second orchestrator and tools do not `finish_turn`.
- Extracting a shared SessionManager start helper is allowed only if the
  controller switches to it with tests proving byte-for-byte the current
  reserve-then-spawn behavior. That extraction is not part of this decision.
