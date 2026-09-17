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
2. **Hub is a completion router.** It subscribes to the same lifecycle stream
   as clients and queues only `(parent_session_id, child_session_id, turn_id)`
   references until the parent reaches a safe injection point. It owns no
   running list, waiter, wire snapshot, or outcome. At delivery time the
   generic Session data API reconstructs `TurnResult` from durable
   `turn/start`, transcript items, and `turn/end`. The report, result path,
   and truncation range therefore remain Session facts rather than a second
   `JobRecord` model. The hub does not join a turn thread or call
   `finish_turn`.
3. **Depth lock is product.** Subagent tools bind only on primary turns
   (`depth == 0` / `SUBAGENT_MAX_DEPTH = 1`). Children cannot nest. There is
   no per-parent concurrency cap.
4. **Session parity.** Child turns use `TurnOptions::agent(...)`; human/idle
   turns use `TurnOptions::default()`. Both go through `spawn_turn`. Busy send
   is the session (`is_turn_running_blocking` / `AgentAlreadyRunning`), not a
   hub latch.
5. **Roster parity.** Responsibility is durable Session metadata. The agent
   roster and human UI derive child state directly from child sessions; there
   is no subagent-specific protocol snapshot or client store.
6. **Wait is event-driven handoff.** `subagent_wait` freezes the selected
   current `turn_id` values and waits for N or all of them to settle. It has no
   duration or polling mode and returns the corresponding durable
   `TurnResult` values.

## Consequences

- `tests/session_turn_entry.rs` and `tests/architecture_layers.rs` pin that
  `src/session/` has no second orchestrator and tools do not `finish_turn`.
- Extracting a shared SessionManager start helper is allowed only if the
  controller switches to it with tests proving byte-for-byte the current
  reserve-then-spawn behavior. That extraction is not part of this decision.
