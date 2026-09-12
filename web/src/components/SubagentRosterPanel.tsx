import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CaretDown } from "@phosphor-icons/react";

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
import { useConnectionStore } from "../stores/connectionStore";
import { displayMessages, useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { useTurnStore } from "../stores/turnStore";
import { MessageList } from "./MessageList";
import { ProgressiveBlur } from "./ProgressiveBlur";
import { releaseSubagentCard } from "./sessionTeardown";
import {
  holdSubagentRoster,
  releaseSubagentRoster,
} from "./subagentRosterHolds";
import { SubagentStatusIcon } from "./SubagentStatusIcon";
import type { ToolStatus } from "./ToolIcon";

const EMPTY_JOBS: SubagentJob[] = [];
const EMPTY_WAITS: SubagentWait[] = [];
const EMPTY_ROWS: HumanRow[] = [];

/**
 * Fixed height of an expanded card's transcript scroller. One scroller ⇒ one
 * virtualizer; the card can never blow out the panel (which owns its own
 * height + overflow). Drag the panel to PANEL_MAX_H to see a card whole.
 */
const CARD_BODY_H = 280;

type FinishedStatus = "completed" | "failed" | "unknown";

interface RosterEntry {
  childId: string;
  callId: string;
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
 * as an expandable card. The roster is sourced from the durable
 * `subagentBindings` (call_id → child session id), deduped by child id, and
 * enriched per child by:
 *   1. the session itself — `sessionStore.sessions` (agent_id / preview /
 *      running) plus the child slice in `sessionStore.byId` (hydrated from the
 *      child's snapshot once it is subscribed). Primary: works with no window.
 *   2. the live `subagentStore.jobs` entry (agent_name + start time → timer).
 *   3. the parent transcript's `subagent_launch` row (agent name / outcome).
 *
 * Expanding a card subscribes the child session and renders its FULL transcript
 * with the same stack as the main panel (MessageList virtualizer + a top
 * ProgressiveBlur); collapsing unsubscribes and drops the child slices — unless
 * the child still has its own dock tab open, which owns that subscription.
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
  const sessionsById = useSessionStore((s) => s.byId);
  const [open, setOpen] = useState<Set<string>>(() => new Set());

  const roster = useMemo(() => {
    const meta = subagentRowMeta(rows ?? EMPTY_ROWS);
    const byChild = new Map<string, RosterEntry>();
    for (const [callId, childId] of Object.entries(bindings ?? {})) {
      if (!childId) continue;
      const job = jobs.find((j) => j.call_id === callId);
      const info = meta.get(callId);
      const session = sessions.find((s) => s.id === childId);
      const agent =
        session?.agent_id ||
        sessionsById.get(childId)?.activePrimary ||
        job?.agent_name ||
        info?.agent;
      const entry: RosterEntry = {
        childId,
        callId,
        agent: agent || undefined,
        running:
          !!job ||
          session?.running === true ||
          session?.status === "running_with_subagent",
        startedAt: job?.started_at_ms,
        finished: info?.finished ?? "unknown",
        preview: session?.preview || undefined,
      };
      // Dedupe by child id — prefer the entry that is still running.
      const prev = byChild.get(childId);
      if (!prev || (entry.running && !prev.running)) byChild.set(childId, entry);
    }
    return [...byChild.values()];
  }, [bindings, rows, jobs, sessions, sessionsById]);

  const toggle = (childId: string) =>
    setOpen((cur) => {
      const next = new Set(cur);
      if (next.has(childId)) next.delete(childId);
      else next.add(childId);
      return next;
    });

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
              open={open.has(entry.childId)}
              onToggle={() => toggle(entry.childId)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function SubagentRosterItem({
  entry,
  open,
  onToggle,
}: {
  entry: RosterEntry;
  open: boolean;
  onToggle: () => void;
}) {
  const label = entry.agent ?? "subagent";
  return (
    <li>
      <button
        type="button"
        aria-expanded={open}
        aria-label={`Subagent ${label}`}
        onClick={onToggle}
        className="flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-primary)"
      >
        <CaretDown
          size={11}
          weight="bold"
          aria-hidden
          className={`shrink-0 transition-transform ${open ? "" : "-rotate-90"}`}
        />
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
      {open && <SubagentCardBody childSessionId={entry.childId} />}
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

/**
 * Subscribes `childSessionId` while the roster card is expanded, then renders
 * the child's FULL transcript with the main-panel stack: a non-scrolling frame
 * carrying the persistent inset, one scroll container (the element the
 * virtualizer measures) and a centered reading-measure column inside it, plus a
 * top ProgressiveBlur tied to the container's scrollTop.
 *
 * On collapse it releases the subscription — but never when the child still has
 * its own `agent-<childId>` dock panel open, since `ensureSubscribe` /
 * `unsubscribeSession` are not refcounted and tearing it down here would kill
 * that tab's stream. While mounted it also HOLDS the session in the shared
 * roster registry, so closing that tab cannot tear down the card's stream
 * either.
 *
 * P6: collapse does NOT drop the child's message/turn slices any more. The
 * slices stay resident (bounded: one per child ever expanded) and a re-expand
 * tops them up through the snapshot gap (`loadRange`) instead of cold-starting.
 */
function SubagentCardBody({ childSessionId }: { childSessionId: string }) {
  const connState = useConnectionStore((s) => s.state);

  useEffect(() => {
    if (connState !== "connected") return;
    holdSubagentRoster(childSessionId);
    void useConnectionStore
      .getState()
      .ensureSubscribe(childSessionId)
      .catch(() => {});
    // `ensureSubscribe` itself starts the catch-up: the child's snapshot carries
    // `buffer.next_seq`, and `sessionStore.applySnapshot` appends the missing
    // tail when the retained window lags it.
    return () => {
      releaseSubagentRoster(childSessionId);
      releaseSubagentCard(childSessionId);
    };
  }, [childSessionId, connState]);

  const messages = useMessageStore((s) =>
    displayMessages(s.bySession.get(childSessionId)),
  );
  const loadingHistory = useMessageStore(
    (s) => s.bySession.get(childSessionId)?.loadingHistory ?? false,
  );
  const fromSeq = useMessageStore(
    (s) => s.bySession.get(childSessionId)?.fromSeq ?? 0,
  );
  const userDetailBefore = useMessageStore(
    (s) => s.bySession.get(childSessionId)?.userDetailBefore ?? 0,
  );
  const runState = useTurnStore(
    (s) => s.byId.get(childSessionId)?.runState ?? "idle",
  );
  const loadMoreHistoryAction = useMessageStore((s) => s.loadMoreHistory);
  const onLoadMore = useCallback(() => {
    loadMoreHistoryAction(childSessionId);
  }, [loadMoreHistoryAction, childSessionId]);

  const listRef = useRef<HTMLDivElement>(null);
  const [blurOpacity, setBlurOpacity] = useState(0);
  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setBlurOpacity(Math.min(el.scrollTop / 72, 1));
  };

  const isRunning = runState === "running" || runState === "cancelling";
  const canLoadMore = fromSeq > 0;
  const empty = messages.length === 0 && !isRunning;

  // Read-only card: the main panel's editing / reveal / jump affordances are
  // deliberately left at MessageList's safe defaults (noop handlers, null
  // anchor), so a child bubble click is inert instead of opening a rewrite box.
  return (
    <div
      className="relative border-t border-(--_dk-line)"
      style={{ height: CARD_BODY_H }}
      data-testid="subagent-card-body"
    >
      {empty ? (
        <p className="px-3 py-2 text-dk-2xs italic text-(--_dk-text-disabled)">
          Empty subagent session
        </p>
      ) : (
        <>
          {/* 1. Non-scrolling frame: carries the persistent inset. */}
          <div className="flex h-full min-h-0 flex-col bg-(--_dk-editor) px-3 pt-2">
            {/* 2. Scroll container: the element the virtualizer measures. */}
            <div
              ref={listRef}
              onScroll={onScroll}
              className="min-h-0 flex-1 overflow-y-auto bg-(--_dk-editor)"
            >
              {/* 3. Content column: centered reading measure only. */}
              <div className="mx-auto flex w-full max-w-[var(--_dk-prose-measure)] flex-col bg-(--_dk-editor)">
                <MessageList
                  key={childSessionId}
                  messages={messages}
                  loadingHistory={loadingHistory}
                  canLoadMore={canLoadMore}
                  onLoadMore={onLoadMore}
                  userDetailBefore={userDetailBefore}
                  isRunning={isRunning}
                  scrollRef={listRef}
                  sessionId={childSessionId}
                />
              </div>
            </div>
          </div>
          <ProgressiveBlur
            side="top"
            opacity={blurOpacity}
            tintColor="var(--_dk-editor)"
            tint={1}
            height={40}
            strength={4}
            tintCurve={1}
          />
        </>
      )}
    </div>
  );
}
