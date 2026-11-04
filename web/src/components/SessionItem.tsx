import { memo, useEffect, useState } from "react";

import type { SessionInfo } from "../api/types";
import { isWaitingForUserPhase } from "../api/types";
import { useTurnStore } from "../stores/turnStore";
import { sessionTitle } from "../lib/sessionDisplay";
import { SidebarRow } from "./SidebarRow";
import {
  isAwaitingPermission,
  type SessionStatus,
} from "./SessionStatusDot";
import i18n from "../i18n";
import { useDragSource } from "./dnd/useDragSource";
import { useDropTarget } from "./dnd/useDropTarget";
import {
  parseSessionDrag,
  SESSION_DRAG_MIME,
  type SessionDragPayload,
} from "./dnd/sessionDnd";

/**
 * DEBUG: when true, every status bar ignores its real status and alternates
 * between `pending` and `running` on a timer, so the glow animations can be
 * eyeballed and tuned. Flip to `false` (or delete) for production.
 */
const DEBUG_CYCLE = false;
const DEBUG_CYCLE_MS = 6000;
/**
 * Compact "time since last update" relative to `now`, at minute granularity.
 * Rolls up as minutes -> hours -> days -> weeks -> months -> years, rendered as
 * a bare `number + unit` token (m/h/d/w/mo/y). The first minute shows "1m"
 * (driven by a shared 1min ticker in the parent, so no per-item intervals).
 */
export function formatRelative(updatedAt: number, now: number): string {
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
  /** Shared wall-clock tick (ms) from the parent's 1min timer. */
  now: number;
  /**
   * Left padding of the row in px. Rows grouped under a folder header pass a
   * deeper inset so they read as children of that header; the flat list keeps
   * the default.
   */
  indentPx?: number;
  /** This session is what the agent area currently shows. */
  focused?: boolean;
  /** Settled to idle while its transcript was hidden; cleared on focus. */
  unread?: boolean;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  /** A session was dropped onto this row. */
  onDropSession?: (sourceId: string) => void;
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
 *   [status dot] [conversation summary] [live summary] [unread · time <-> delete]
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
  focused = false,
  unread = false,
  onOpen,
  onDelete,
  onDropSession,
  onContextMenu,
  renaming = false,
  onStartRename,
  onRename,
  onRenameCancel,
}: SessionItemProps) {
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
    : i18n.t("session.deleteSession");
  const status: SessionStatus = pending
    ? "pending"
    : deleteBlocked
      ? "running"
      : "idle";

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

  const label = sessionTitle(session);
  const relative = formatRelative(session.updated_at, now);
  const drag = useDragSource<SessionDragPayload>({
    mime: SESSION_DRAG_MIME,
    payload: () => ({ kind: "session", id: session.id }),
    canDrag: !renaming,
    effect: "move",
    text: (payload) => payload.id,
  });
  const drop = useDropTarget<SessionDragPayload>({
    mime: SESSION_DRAG_MIME,
    parse: parseSessionDrag,
    onDrop: (payload) => onDropSession?.(payload.id),
  });

  return (
    <SidebarRow
      label={label}
      title={label}
      status={shown}
      unread={unread}
      focused={focused}
      relative={relative}
      stepKinds={session.step_kinds ?? []}
      running={session.running}
      updatedAt={session.updated_at}
      now={now}
      indentPx={indentPx}
      drag={drag}
      dropHandlers={drop.handlers}
      dropPosition={drop.position}
      dataAttrs={{
        "data-session-active": focused ? "true" : undefined,
        "data-session-unread": unread ? "true" : undefined,
      }}
      onOpen={() => onOpen(session.id)}
      onDelete={() => onDelete(session.id)}
      deleteBlocked={deleteBlocked}
      deleteLabel={deleteLabel}
      onContextMenu={onContextMenu}
      renaming={renaming}
      renameInitialValue={session.title ?? ""}
      renameAriaLabel={i18n.t("session.sessionTitle")}
      renameMaxLength={200}
      renameAllowEmpty
      onStartRename={onStartRename}
      onRename={(value) => onRename?.(value || null)}
      onRenameCancel={() => onRenameCancel?.()}
    />
  );
});
