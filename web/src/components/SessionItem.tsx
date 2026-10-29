import { memo, useEffect, useRef, useState } from "react";
import { Trash } from "@phosphor-icons/react";
import type { SessionInfo } from "../api/types";
import { isWaitingForUserPhase } from "../api/types";
import { useTurnStore } from "../stores/turnStore";
import { LivePreview } from "./LivePreview";
import {
  SessionStatusDot,
  isAwaitingPermission,
  type SessionStatus,
} from "./SessionStatusDot";
import i18n from "../i18n";

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

/** MIME used for dragging a session row onto a folder header. */
export const SESSION_DRAG_MIME = "application/x-litecode-session-id";

interface SessionItemProps {
  session: SessionInfo;
  /** Shared wall-clock tick (ms) from the parent's 1s timer. */
  now: number;
  /**
   * Left padding of the row in px. Rows grouped under a folder header pass a
   * deeper inset so they read as children of that header; the flat list keeps
   * the default.
   */
  indentPx?: number;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  onContextMenu?: (event: React.MouseEvent) => void;
  /** Inline title editor is open for this row. */
  renaming?: boolean;
  /** Open the inline title editor (double-click on the title). */
  onStartRename?: () => void;
  /** Commit the draft; `null` clears the override and restores the preview. */
  onRename?: (title: string | null) => void;
  onRenameCancel?: () => void;
}

/**
 * Pure presentational row for a single session. Single-line layout:
 *   [status dot] [conversation summary] [live summary] [time ⇄ delete]
 * Owns no store subscriptions — `session` + `now` are supplied by the parent.
 * The `pending` status (WP-184 / WP-186): WireTurnPhase `waiting_for_user`
 * (legacy object key `awaiting_permission`), else trust BE
 * `turn.awaiting_permission` bool; if that field is absent, derive from local
 * `permissionQueue` length.
 */
export const SessionItem = memo(function SessionItem({
  session,
  now,
  indentPx = 12,
  onOpen,
  onDelete,
  onContextMenu,
  renaming = false,
  onStartRename,
  onRename,
  onRenameCancel,
}: SessionItemProps) {
  // WP-184 Phase B: phase object OR bool / local queue derivation.
  const pendingCount = useTurnStore(
    (s) => s.byId.get(session.id)?.permissionQueue.length ?? 0,
  );
  const pending =
    isWaitingForUserPhase(session.turn?.phase) ||
    isAwaitingPermission(session.turn?.awaiting_permission, pendingCount);
  const subagentBusy =
    session.status === "running_with_subagent" || session.status === "stopping";
  const deleteBlocked = session.running || subagentBusy;
  const deleteLabel = deleteBlocked
    ? i18n.t("session.cannotDeleteBusy")
    : "Delete session";
  const status: SessionStatus = pending
    ? "pending"
    : deleteBlocked
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
  const shown: SessionStatus = DEBUG_CYCLE
    ? toggled
      ? "running"
      : "pending"
    : status;

  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const renameDoneRef = useRef(false);
  useEffect(() => {
    if (!renaming) return;
    renameDoneRef.current = false;
    setDraft(session.title ?? "");
    const raf = requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => cancelAnimationFrame(raf);
  }, [renaming, session.title]);
  const finishRename = (value: string) => {
    if (renameDoneRef.current) return;
    renameDoneRef.current = true;
    const title = value.trim();
    onRename?.(title || null);
  };

  const preview = session.preview?.trim();
  const title = session.title?.trim();
  const label = title || preview;
  const relative = formatRelative(session.updated_at, now);

  return (
    <div className="group relative h-9">
      <div
        className="flex h-9 cursor-pointer items-center gap-2 pr-12 text-(--_dk-ix-fg) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary) active:bg-(--_dk-ix-bg-pressed) focus-visible:shadow-[0_0_0_2px_var(--_dk-ix-ring)]"
        style={{ paddingLeft: indentPx }}
        draggable={!renaming}
        onDragStart={(event) => {
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData(SESSION_DRAG_MIME, session.id);
          event.dataTransfer.setData("text/plain", session.id);
        }}
        onContextMenu={onContextMenu}
        onClick={(event) => {
          // A double-click starts renaming; do not re-open on its second click.
          if (!renaming && event.detail < 2) onOpen(session.id);
        }}
      >
        {/* Middle group: status bar + glow (rendered by SessionStatusDot, anchored
            to this content) followed by the conversation preview and the live
            summary glyph. The glow is pinned to the content's right edge so it can
            never overrun into the live-summary glyphs. */}
        <SessionStatusDot status={shown}>
          {/* Conversation summary — adapts to content width, capped + truncated.
              `min-w-0` lets it shrink below content as the row narrows. */}
          {renaming ? (
            <input
              ref={inputRef}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onClick={(event) => event.stopPropagation()}
              onMouseDown={(event) => event.stopPropagation()}
              onKeyDown={(event) => {
                // Keep Enter/Escape inside the editor; global shortcuts must
                // not fire while the user is naming a session.
                event.stopPropagation();
                if (event.key === "Enter") {
                  event.preventDefault();
                  finishRename(draft);
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  renameDoneRef.current = true;
                  onRenameCancel?.();
                }
              }}
              onBlur={() => finishRename(draft)}
              aria-label={i18n.t("session.sessionTitle")}
              maxLength={200}
              className="w-28 min-w-0 rounded border border-(--_dk-ix-ring) bg-(--_dk-editor) px-1 py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none"
            />
          ) : (
            <span
              className="max-w-32 min-w-0 text-(--_dk-text-secondary)"
              title={preview || title || undefined}
              onDoubleClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                onStartRename?.();
              }}
            >
              <span className="block truncate text-[11px]">
                {label || (
                  <span className="text-(--_dk-text-disabled)">No preview</span>
                )}
              </span>
            </span>
          )}
          {/* Live summary — real-time step label when active, else a triggered emoji. */}
          <LivePreview
            stepKinds={session.step_kinds ?? []}
            running={session.running}
            updatedAt={session.updated_at}
            now={now}
          />
        </SessionStatusDot>
      </div>
      {/* Time while idle; the delete square takes its place on hover. */}
      <span className="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-3 text-[10px] text-(--_dk-text-muted) transition-opacity group-hover:opacity-0">
        {relative}
      </span>
      {/* Row-height square delete target, flush to the row's right edge. It is a
          sibling of the clickable row, so hovering it never paints the row —
          only its own borderless danger wash shows. */}
      <button
        type="button"
        disabled={deleteBlocked}
        onClick={(e) => {
          e.stopPropagation();
          if (!deleteBlocked) onDelete(session.id);
        }}
        aria-label={deleteLabel}
        className="btn-danger btn-icon pointer-events-none absolute right-0 top-0 h-9 w-9 p-0 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-0 disabled:group-hover:opacity-40"
        title={deleteLabel}
      >
        <Trash size={14} aria-hidden />
      </button>
    </div>
  );
});
