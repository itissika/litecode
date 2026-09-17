import { useEffect, useMemo, useRef, useState } from "react";

import {
  functionCallOutputText,
  isFunctionCall,
  isFunctionCallOutput,
  itemFromRow,
  parseFunctionArguments,
} from "../api/adapter";
import type {
  HumanRow,
  SubagentJob,
  SubagentWait,
} from "../api/types";
import { formatElapsed } from "../lib/bashLive";
import { openSubagentPanel } from "../lib/sessionPanelNav";
import { useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { SubagentStatusIcon } from "./SubagentStatusIcon";
import type { ToolStatus } from "./ToolIcon";

const EMPTY_JOBS: SubagentJob[] = [];
const EMPTY_WAITS: SubagentWait[] = [];
const EMPTY_ROWS: HumanRow[] = [];

type FinishedStatus = "completed" | "failed" | "unknown" | "finished";
interface RosterEntry {
  childId: string;
  /** May be unknown until the session row (or a live job) names it. */
  callId?: string;
  /** Resolved agent type; undefined until some source knows it. */
  agent?: string;
  running: boolean;
  startedAt?: number;
  finished: FinishedStatus;
  /** Latest-message preview of the child session, when the list has it. */
  preview?: string;
}

/**
 * Per-call metadata for subagent launches, reconstructed from the parent's
 * already-loaded transcript rows (durable across a finished job). Returns
 * call_id → { agent, finished } so a bound child can be labelled after its
 * runtime job has left the store without pulling any extra history.
 *
 * This is the FALLBACK path: the primary agent/status source is the session
 * itself (see the roster memo), which keeps working when the parent launch row
 * has scrolled out of the loaded window.
 */
function subagentRowMeta(
  rows: HumanRow[],
): Map<string, { agent?: string; finished: FinishedStatus }> {
  const calls = new Map<string, string | undefined>();
  const outputs = new Map<string, string>();
  for (const row of rows) {
    const item = itemFromRow(row);
    if (!item) continue;
    if (isFunctionCall(item)) {
      if (item.name !== "subagent_launch" || !item.call_id) continue;
      const args = parseFunctionArguments(item.arguments);
      const agent =
        args &&
        typeof args === "object" &&
        !Array.isArray(args) &&
        typeof (args as Record<string, unknown>).agent === "string"
          ? ((args as Record<string, unknown>).agent as string)
          : undefined;
      calls.set(item.call_id, agent);
    } else if (isFunctionCallOutput(item) && item.call_id) {
      outputs.set(item.call_id, functionCallOutputText(item));
    }
  }
  const meta = new Map<string, { agent?: string; finished: FinishedStatus }>();
  for (const [callId, agent] of calls) {
    const text = outputs.get(callId);
    const finished: FinishedStatus =
      text === undefined
        ? "unknown"
        : text.startsWith("Error:")
          ? "failed"
          : "completed";
    meta.set(callId, { agent, finished });
  }
  return meta;
}

/** Sealed tool status for the presence icon (only read once the child settles). */
function iconStatus(entry: RosterEntry): ToolStatus {
  if (entry.running) return "running";
  if (entry.finished === "failed") return "failed";
  if (entry.finished === "completed") return "ok";
  return "unknown";
}

/**
 * Dock "Workers" panel body: every subagent session launched from this parent,
 * as a flat row. The roster is sourced from the durable `subagentBindings`
 * (call_id → child session id), deduped by child id, and enriched per child by:
 *   1. the session itself — `sessionStore.sessions` (agent_id / assistant_preview /
 *      preview / running). PRIMARY: the server lists child sessions too, so this
 *      labels a child that was never subscribed (P7). `sessionStore.byId` (the
 *      child slice, hydrated once subscribed) backs it up.
 *   2. the live `subagentStore.jobs` entry (agent_name + start time → timer).
 *   3. the parent transcript's `subagent_launch` row (agent name / outcome).
 *
 * A row is pure NAVIGATION: clicking it opens/focuses the child's independent
 * read-only dock panel (`openSubagentPanel`). The roster no longer embeds the
 * child transcript, holds a child subscription, or owns any per-row expansion
 * state — that surface lives entirely in `SubagentReadOnlyPanel`.
 */
export function SubagentRosterPanel({ sessionId }: { sessionId: string }) {
  const bindings = useMessageStore(
    (s) => s.bySession.get(sessionId)?.subagentBindings,
  );
  const rows = useMessageStore((s) => s.bySession.get(sessionId)?.display);
  const jobs = useSubagentStore(
    (s) => s.bySession.get(sessionId)?.jobs ?? EMPTY_JOBS,
  );
  const waits = useSubagentStore(
    (s) => s.bySession.get(sessionId)?.waits ?? EMPTY_WAITS,
  );
  const sessions = useSessionStore((s) => s.sessions);
  const listSessions = useSessionStore((s) => s.listSessions);
  const connState = useConnectionStore((s) => s.state);
  // `sessionStore.sessions` is globally sorted by updated_at, so live children
  // can trade places on every lifecycle refresh. Preserve first-seen order while
  // this panel is mounted; otherwise a row moves under a stationary pointer
  // during scrolling and the next click can hit a different row.
  const rosterOrderRef = useRef<Map<string, number>>(new Map());

  // `session/list` is only PUSHED on session create/delete — never on turn
  // start/finish or a preview update — so the list would go stale while the
  // panel is closed. Re-pull it on open (same gating as SessionList, so the
  // request never races the socket handshake).
  useEffect(() => {
    if (connState !== "connected") return;
    listSessions();
  }, [connState, listSessions]);

  const roster = useMemo(() => {
    const meta = subagentRowMeta(rows ?? EMPTY_ROWS);
    const byChild = new Map<string, RosterEntry>();

    // PRIMARY — the session list is the lifecycle-matched source: a subagent
    // child exists in it exactly as its parent does (pushed on create/delete,
    // re-pulled on open and after `agent/subagent_bound`), so the roster
    // survives a reload instead of dying with the ephemeral bindings map.
    for (const s of sessions) {
      if (s.parent_session_id !== sessionId) continue;
      const callId = s.parent_call_id ?? undefined;
      const job = callId ? jobs.find((j) => j.call_id === callId) : undefined;
      const info = callId ? meta.get(callId) : undefined;
      byChild.set(s.id, {
        childId: s.id,
        callId,
        agent: s.agent_id || job?.agent_name || info?.agent || undefined,
        running:
          s.running === true ||
          s.status === "running_with_subagent" ||
          !!job,
        startedAt: job?.started_at_ms,
        finished: info ? info.finished : "finished",
        preview: s.assistant_preview || s.preview || undefined,
      });
    }

    // TRANSIENT — a child whose binding/job landed before its session row has
    // arrived in a (re)pulled list (the async gap right after subagent_bound).
    for (const [callId, childId] of Object.entries(bindings ?? {})) {
      if (!childId || byChild.has(childId)) continue;
      const job = jobs.find((j) => j.call_id === callId);
      const info = meta.get(callId);
      const session = sessions.find((s) => s.id === childId);
      byChild.set(childId, {
        childId,
        callId,
        agent: session?.agent_id || job?.agent_name || info?.agent || undefined,
        running:
          !!job ||
          session?.running === true ||
          session?.status === "running_with_subagent",
        startedAt: job?.started_at_ms,
        finished: info ? info.finished : "finished",
        preview: session?.assistant_preview || session?.preview || undefined,
      });
    }
    const next = [...byChild.values()];
    for (const entry of next) {
      if (!rosterOrderRef.current.has(entry.childId)) {
        rosterOrderRef.current.set(entry.childId, rosterOrderRef.current.size);
      }
    }
    return next.sort(
      (a, b) =>
        rosterOrderRef.current.get(a.childId)! -
        rosterOrderRef.current.get(b.childId)!,
    );
  }, [bindings, rows, jobs, sessions, sessionId]);

  if (roster.length === 0 && waits.length === 0) {
    return (
      <div className="px-3 py-2 text-xs italic text-(--_dk-text-disabled)">
        No subagents
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-0.5">
      {waits.length > 0 && (
        <ul className="space-y-0.5 px-1.5" data-testid="subagent-roster-waits">
          {waits.map((wait) => (
            <li
              key={wait.call_id}
              className="px-1.5 py-1 font-mono text-dk-xs text-(--_dk-text-muted)"
            >
              waiting on {wait.watching_id ?? "subagent"}
            </li>
          ))}
        </ul>
      )}
      {roster.length > 0 && (
        <ul className="space-y-0.5 px-1.5" data-testid="subagent-roster">
          {roster.map((entry) => (
            <SubagentRosterItem
              key={entry.childId}
              entry={entry}
              onOpen={() => openSubagentPanel(entry.childId)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function SubagentRosterItem({
  entry,
  onOpen,
}: {
  entry: RosterEntry;
  onOpen: () => void;
}) {
  const label = entry.agent ?? "subagent";
  return (
    <li>
      <button
        type="button"
        aria-label={`Subagent ${label}`}
        onClick={onOpen}
        className="flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-primary)"
      >
        <SubagentStatusIcon
          agent={entry.agent}
          live={entry.running}
          status={iconStatus(entry)}
          runState={entry.running ? "running" : "idle"}
        />
        <span
          data-testid="subagent-roster-agent"
          className="shrink-0 font-mono text-(--_dk-text-primary)"
        >
          {label}
        </span>
        {entry.preview ? (
          <span
            data-testid="subagent-roster-preview"
            className="min-w-0 flex-1 truncate text-(--_dk-text-muted)"
          >
            {entry.preview}
          </span>
        ) : null}
        <SubagentStatus entry={entry} />
      </button>
    </li>
  );
}

/** Running → live timer; finished → the sealed status word. */
function SubagentStatus({ entry }: { entry: RosterEntry }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!entry.running) return;
    const t = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(t);
  }, [entry.running]);

  if (entry.running) {
    return (
      <span
        className="ml-auto shrink-0 font-mono text-(--_dk-text-muted)"
        data-testid="subagent-roster-running"
      >
        {entry.startedAt !== undefined
          ? `running ${formatElapsed(now - entry.startedAt)}`
          : "running"}
      </span>
    );
  }
  const text =
    entry.finished === "failed"
      ? "failed"
      : entry.finished === "completed"
        ? "completed"
        : entry.finished === "finished"
          ? "finished"
          : "unknown";
  return (
    <span
      className={`ml-auto shrink-0 ${
        entry.finished === "failed"
          ? "text-(--_dk-red-500)"
          : "text-(--_dk-text-muted)"
      }`}
      data-testid="subagent-roster-finished"
    >
      {text}
    </span>
  );
}
