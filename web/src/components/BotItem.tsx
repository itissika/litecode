import { Robot } from "@phosphor-icons/react";

import type { BotInfo, SessionInfo } from "../api/types";
import { isWaitingForUserPhase } from "../api/types";
import { useTurnStore } from "../stores/turnStore";
import { botParentId } from "../lib/botDirectory";
import { SidebarRow } from "./SidebarRow";
import { formatRelative } from "./SessionItem";
import {
  isAwaitingPermission,
  type SessionStatus,
} from "./SessionStatusDot";
import i18n from "../i18n";
import { useDragSource } from "./dnd/useDragSource";
import { useDropTarget } from "./dnd/useDropTarget";
import {
  BOT_DRAG_MIME,
  parseBotDrag,
  type BotDragPayload,
} from "./dnd/sessionDnd";

export interface BotItemProps {
  bot: BotInfo;
  /** The bot's ordinary default session, when the session list has it. */
  session?: SessionInfo;
  now: number;
  indentPx?: number;
  focused?: boolean;
  unread?: boolean;
  onOpen: (id: string) => void;
  onDelete: (bot: BotInfo) => void;
  /** A bot was dropped onto this row. */
  onDropBot?: (sourceId: string) => void;
  onContextMenu?: (event: React.MouseEvent) => void;
  renaming?: boolean;
  onStartRename?: () => void;
  onRename?: (name: string) => void;
  onRenameCancel?: () => void;
}

/** One bot row. It reuses the session row's layout, status, and interactions. */
export function BotItem({
  bot,
  session,
  now,
  indentPx,
  focused = false,
  unread = false,
  onOpen,
  onDelete,
  onDropBot,
  onContextMenu,
  renaming = false,
  onStartRename,
  onRename,
  onRenameCancel,
}: BotItemProps) {
  const pendingCount = useTurnStore((s) =>
    session
      ? (s.byId.get(session.id)?.permissionQueue.length ?? 0)
      : 0,
  );
  const pending =
    isWaitingForUserPhase(session?.turn?.phase) ||
    isAwaitingPermission(session?.turn?.awaiting_permission, pendingCount);
  const subagentBusy =
    session?.status === "running_with_subagent" ||
    session?.status === "stopping";
  const deleteBlocked = Boolean(session?.running || subagentBusy);
  const status: SessionStatus = pending
    ? "pending"
    : deleteBlocked
      ? "running"
      : "idle";
  const updatedAt = session?.updated_at ?? bot.created_at;
  const relative = formatRelative(updatedAt, now);
  const deleteLabel = deleteBlocked
    ? i18n.t("bot.cannotDeleteBusy")
    : i18n.t("bot.delete");
  const drag = useDragSource<BotDragPayload>({
    mime: BOT_DRAG_MIME,
    payload: () => ({ kind: "bot", id: bot.id }),
    canDrag: !renaming,
    effect: "move",
    text: (payload) => payload.id,
  });
  const drop = useDropTarget<BotDragPayload>({
    mime: BOT_DRAG_MIME,
    parse: parseBotDrag,
    onDrop: (payload) => onDropBot?.(payload.id),
  });

  return (
    <SidebarRow
      label={bot.name}
      title={bot.description || bot.name}
      status={status}
      unread={unread}
      focused={focused}
      relative={relative}
      stepKinds={session?.step_kinds ?? []}
      running={Boolean(session?.running)}
      updatedAt={updatedAt}
      now={now}
      indentPx={indentPx}
      icon={<Robot size={13} className="shrink-0 text-(--_dk-text-secondary)" />}
      drag={drag}
      dropHandlers={drop.handlers}
      dropPosition={drop.position}
      dataAttrs={{
        "data-bot-active": focused ? "true" : undefined,
        "data-bot-unread": unread ? "true" : undefined,
        "data-bot-parent": botParentId(bot) ?? undefined,
      }}
      onOpen={() => onOpen(bot.id)}
      onDelete={() => onDelete(bot)}
      deleteBlocked={deleteBlocked}
      deleteLabel={deleteLabel}
      onContextMenu={onContextMenu}
      renaming={renaming}
      renameInitialValue={bot.name}
      renameAriaLabel={i18n.t("bot.name")}
      renameMaxLength={48}
      onStartRename={onStartRename}
      onRename={(value) => onRename?.(value)}
      onRenameCancel={() => onRenameCancel?.()}
    />
  );
}
