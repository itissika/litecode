import type { MouseEvent } from "react";
import { CaretDown, CaretRight, FolderSimple } from "@phosphor-icons/react";

import type { SessionFolder } from "../api/types";
import i18n from "../i18n";
import { useDragSource } from "./dnd/useDragSource";
import { useDropTarget } from "./dnd/useDropTarget";
import {
  BOT_DRAG_MIME,
  parseBotDrag,
  parseSessionDrag,
  parseSessionFolderDrag,
  SESSION_DRAG_MIME,
  SESSION_FOLDER_DRAG_MIME,
  type BotDragPayload,
  type SessionDragPayload,
  type SessionFolderDragPayload,
} from "./dnd/sessionDnd";
import { InlineRename } from "./ui/InlineRename";
import { ListRow } from "./ui/ListRow";

type FolderDropPayload =
  | SessionDragPayload
  | BotDragPayload
  | SessionFolderDragPayload;

interface SessionFolderHeaderProps {
  folder: SessionFolder;
  count: number;
  collapsed: boolean;
  renaming: boolean;
  onToggle: () => void;
  onContextMenu: (event: MouseEvent) => void;
  onRename: (name: string) => void;
  onRenameCancel: () => void;
  onDropSession: (sessionId: string) => void;
  onDropBot: (botId: string) => void;
  onReorderFolder: (
    sourceId: string,
    targetId: string,
    position: "before" | "after",
  ) => void;
  onMoveFolder: (id: string, delta: -1 | 1) => void;
}

/** Folder row: collapse, inline rename, session drop, and folder reorder. */
export function SessionFolderHeader({
  folder,
  count,
  collapsed,
  renaming,
  onToggle,
  onContextMenu,
  onRename,
  onRenameCancel,
  onDropSession,
  onDropBot,
  onReorderFolder,
  onMoveFolder,
}: SessionFolderHeaderProps) {
  const drag = useDragSource<SessionFolderDragPayload>({
    mime: SESSION_FOLDER_DRAG_MIME,
    payload: () => ({ kind: "session-folder", id: folder.id }),
    canDrag: !renaming,
    effect: "move",
    text: (payload) => payload.id,
  });

  const drop = useDropTarget<FolderDropPayload>([
    {
      mime: SESSION_DRAG_MIME,
      parse: parseSessionDrag,
      onDrop: (payload) => {
        if (payload.kind === "session") onDropSession(payload.id);
      },
    },
    {
      mime: BOT_DRAG_MIME,
      parse: parseBotDrag,
      onDrop: (payload) => {
        if (payload.kind === "bot") onDropBot(payload.id);
      },
    },
    {
      mime: SESSION_FOLDER_DRAG_MIME,
      parse: parseSessionFolderDrag,
      resolve: (event, rect) =>
        event.clientY < rect.top + rect.height / 2 ? "before" : "after",
      canDrop: (payload) => payload.id !== folder.id,
      onDrop: (payload, position) => {
        if (payload.kind === "session-folder" && position !== "inside") {
          onReorderFolder(payload.id, folder.id, position);
        }
      },
    },
  ]);

  return (
    <ListRow
      as="div"
      size="section"
      role="button"
      tabIndex={0}
      aria-expanded={!collapsed}
      aria-label={`${
        collapsed
          ? i18n.t("session.expandFolder")
          : i18n.t("session.collapseFolder")
      }: ${folder.name}`}
      {...drag}
      {...drop.handlers}
      onClick={onToggle}
      onKeyDown={(event) => {
        if (
          event.altKey &&
          (event.key === "ArrowUp" || event.key === "ArrowDown")
        ) {
          event.preventDefault();
          event.stopPropagation();
          onMoveFolder(folder.id, event.key === "ArrowUp" ? -1 : 1);
          return;
        }
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onToggle();
        }
      }}
      onContextMenu={onContextMenu}
      drop={drop.position}
      className={`cursor-pointer text-(--_dk-text-secondary) ${
        drop.isOver ? "" : "hover:bg-(--_dk-ix-bg-hover)"
      }`}
    >
      {collapsed ? (
        <CaretRight size={10} className="shrink-0 text-(--_dk-text-muted)" />
      ) : (
        <CaretDown size={10} className="shrink-0 text-(--_dk-text-muted)" />
      )}
      <FolderSimple size={13} className="shrink-0 text-(--_dk-text-muted)" />
      {renaming ? (
        <InlineRename
          initialValue={folder.name}
          ariaLabel={i18n.t("session.folderName")}
          maxLength={120}
          onCommit={(name) => {
            if (name === folder.name) onRenameCancel();
            else onRename(name);
          }}
          onCancel={onRenameCancel}
          className="min-w-0 flex-1 rounded border border-(--_dk-ix-ring) bg-(--_dk-editor) px-1 py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none"
        />
      ) : (
        <span className="min-w-0 flex-1 truncate">{folder.name}</span>
      )}
      <span className="ml-auto shrink-0 text-[10px] text-(--_dk-text-disabled)">
        {count}
      </span>
    </ListRow>
  );
}
