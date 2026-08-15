import { memo, useEffect, useState } from "react";
import type { SessionInfo } from "../api/types";
import { LivePreview } from "./LivePreview";
import { SessionStatusDot, type SessionStatus } from "./SessionStatusDot";

/**
 * DEBUG: when true, every status bar ignores its real status and alternates
 * between `pending` and `running` on a timer, so the glow animations can be
 * eyeballed and tuned. Flip to `false` (or delete) for production.
 */
const DEBUG_CYCLE = false;
const DEBUG_CYCLE_MS = 6000;
/**
 * Compact "time since last update" relative to `now`, at minute granularity.
 * Rolls up as minutes → hours → days → weeks → months → years, rendered as a
 * bare `number + unit` token (m/h/d/w/mo/y). The first minute shows "1m"
 * (driven by a shared 1min ticker in the parent, so no per-item intervals).
 */
function formatRelative(updatedAt: number, now: number): string {
  const diffSec = Math.max(0, Math.floor((now - (updatedAt || now)) / 1000));
  const min = Math.max(1, Math.floor(diffSec / 60));
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d`;
  const wk = Math.floor(day / 7);
  if (wk < 5) return `${wk}w`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo`;
  const yr = Math.floor(day / 365);
  return `${yr}y`;
}

interface SessionItemProps {
  session: SessionInfo;
  /** Shared wall-clock tick (ms) from the parent's 1s timer. */
  now: number;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
}

/**
 * Pure presentational row for a single session. Single-line layout:
 *   [status dot] [conversation summary] [live summary] [time ⇄ delete]
 * Owns no store subscriptions — `session` + `now` are supplied by the parent.
 * The `pending` (awaiting a permission grant) status is read directly from the
 * session payload (`session.turn?.awaiting_permission`), so it reflects every
 * session in the list — including ones not currently open.
 */
export const SessionItem = memo(function SessionItem({
  session,
  now,
  onOpen,
  onDelete,
}: SessionItemProps) {
  const pending = session.turn?.awaiting_permission ?? false;
  const status: SessionStatus = pending
    ? "pending"
    : session.running
      ? "running"
      : "idle";

  // DEBUG: when on, alternate the shown status so the glow animations can be
  // eyeballed. Lifted here so the vertical bar and the glow share one state.
  const [toggled, setToggled] = useState(false);
  useEffect(() => {
    if (!DEBUG_CYCLE) return;
    const id = setInterval(() => setToggled((v) => !v), DEBUG_CYCLE_MS);
    return () => clearInterval(id);
  }, []);
  const shown: SessionStatus = DEBUG_CYCLE ? (toggled ? "running" : "pending") : status;

  const preview = session.preview?.trim();
  const relative = formatRelative(session.updated_at, now);

  return (
    <div
      className="group flex h-9 cursor-pointer items-center gap-2 px-3 text-(--_dk-ix-fg) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary) active:bg-(--_dk-ix-bg-pressed) focus-visible:shadow-[0_0_0_2px_var(--_dk-ix-ring)]"
      onClick={() => onOpen(session.id)}
    >
      {/* Middle group: status bar + glow (rendered by SessionStatusDot, anchored
          to this content) followed by the conversation preview and the live
          summary glyph. The glow is pinned to the content's right edge so it can
          never overrun into the live-summary glyphs. */}
      <SessionStatusDot status={shown}>
        {/* Conversation summary — adapts to content width, capped + truncated.
            `min-w-0` lets it shrink below content as the row narrows. */}
        <span
          className="max-w-32 min-w-0 text-(--_dk-text-secondary)"
          title={preview || undefined}
        >
          <span className="block truncate text-[11px]">
            {preview || <span className="text-(--_dk-text-disabled)">No preview</span>}
          </span>
        </span>
        {/* Live summary — real-time step label when active, else a triggered emoji. */}
        <LivePreview
          stepKinds={session.step_kinds ?? []}
          running={session.running}
          updatedAt={session.updated_at}
          now={now}
        />
      </SessionStatusDot>
      {/* Right slot: time by default, delete button on hover (shared slot).
          `min-w-0` (dropping `shrink-0`) lets it collapse to near-zero at extreme
          narrow widths so the row can shrink until only the time glyph remains. */}
      <div className="relative flex h-4 w-10 min-w-0 items-center justify-end">
        <span className="pointer-events-none absolute inset-0 flex items-center justify-end text-[10px] text-(--_dk-text-muted) transition-opacity group-hover:opacity-0">
          {relative}
        </span>
        <button
          type="button"
          disabled={session.running}
          onClick={(e) => {
            e.stopPropagation();
            if (!session.running) onDelete(session.id);
          }}
          aria-label={session.running ? "Cannot delete while running" : "Delete session"}
          className="pointer-events-none absolute inset-0 flex items-center justify-end text-[11px] text-(--_dk-text-muted) opacity-0 transition-opacity hover:text-(--_dk-ix-danger-fg) group-hover:pointer-events-auto group-hover:opacity-100 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-0 disabled:group-hover:opacity-40"
          title={session.running ? "Cannot delete while running" : "Delete session"}
        >
          ✕
        </button>
      </div>
    </div>
  );
});
