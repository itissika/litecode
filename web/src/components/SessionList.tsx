import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type MouseEvent,
} from "react";
import {
  CaretDown,
  CaretRight,
  FolderPlus,
  Info,
  StackSimple,
} from "@phosphor-icons/react";

import type { SessionInfo } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { SessionItem, SESSION_DRAG_MIME } from "./SessionItem";
import { SessionFolderHeader } from "./SessionFolderHeader";
import { FileTreeContextMenu, type FileTreeMenuItem } from "./FileTreeContextMenu";
import { Popover } from "./ui/Popover";
import { WorkspaceUsage } from "./WorkspaceUsage";
import { openSessionPanel } from "../lib/sessionPanelNav";
import i18n from "../i18n";

const COLLAPSED_KEY = "litecode.sessionFolders.collapsed";
const UNGROUPED_KEY = "__ungrouped__";
/**
 * Grouped rows sit under their header's name. A folder header's label starts at
 * 43px (px-2 8 + caret 10 + gap 6 + folder glyph 13 + gap 6); a session row
 * leads with its 2px status bar plus an 8px gap, so padding the row by 33px
 * lands its text on the folder name. The flat, headerless list keeps 12px.
 */
const FOLDER_LABEL_INSET_PX = 43;
const SESSION_ROW_LEADING_PX = 15;
const GROUPED_ROW_INDENT_PX = FOLDER_LABEL_INSET_PX - SESSION_ROW_LEADING_PX;

type MenuState =
  | { kind: "session"; id: string; x: number; y: number }
  | { kind: "folder"; id: string; x: number; y: number };

function loadCollapsed(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object"
      ? (parsed as Record<string, boolean>)
      : {};
  } catch {
    return {};
  }
}

function UngroupedHeader({
  count,
  collapsed,
  dragOver,
  onToggle,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  count: number;
  collapsed: boolean;
  dragOver: boolean;
  onToggle: () => void;
  onDragOver: (event: DragEvent) => void;
  onDragLeave: (event: DragEvent) => void;
  onDrop: (event: DragEvent) => void;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      aria-expanded={!collapsed}
      aria-label={`${
        collapsed
          ? i18n.t("session.expandFolder")
          : i18n.t("session.collapseFolder")
      }: ${i18n.t("session.ungrouped")}`}
      onClick={onToggle}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onToggle();
        }
      }}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`group flex h-8 cursor-pointer items-center gap-1.5 rounded px-2 text-[11px] text-(--_dk-text-secondary) transition-colors ${
        dragOver
          ? "bg-(--_dk-ix-bg-selected) shadow-[0_0_0_1px_var(--_dk-ix-ring)]"
          : "hover:bg-(--_dk-ix-bg-hover)"
      }`}
    >
      {collapsed ? (
        <CaretRight size={10} className="shrink-0 text-(--_dk-text-muted)" />
      ) : (
        <CaretDown size={10} className="shrink-0 text-(--_dk-text-muted)" />
      )}
      <StackSimple size={13} className="shrink-0 text-(--_dk-text-muted)" />
      <span className="min-w-0 flex-1 truncate">
        {i18n.t("session.ungrouped")}
      </span>
      <span className="ml-auto shrink-0 text-[10px] text-(--_dk-text-disabled)">
        {count}
      </span>
    </div>
  );
}

export function SessionList() {
  const sessions = useSessionStore((s) => s.sessions);
  const folders = useSessionStore((s) => s.folders);
  const loading = useSessionStore((s) => s.sessionsLoading);
  const error = useSessionStore((s) => s.sessionListError);
  const deleteSession = useSessionStore((s) => s.deleteSession);
  const newSession = useSessionStore((s) => s.newSession);
  const listSessions = useSessionStore((s) => s.listSessions);
  const renameSession = useSessionStore((s) => s.renameSession);
  const moveSession = useSessionStore((s) => s.moveSession);
  const createFolder = useSessionStore((s) => s.createFolder);
  const renameFolder = useSessionStore((s) => s.renameFolder);
  const deleteFolder = useSessionStore((s) => s.deleteFolder);
  const connState = useConnectionStore((s) => s.state);

  // The server also lists subagent CHILD sessions (the roster needs them to
  // label a child that was never subscribed), so filter the sidebar to roots.
  const roots = useMemo(
    () => sessions.filter((s) => !s.parent_session_id),
    [sessions],
  );
  const orderedFolders = useMemo(
    () => [...folders].sort((a, b) => a.sort_order - b.sort_order),
    [folders],
  );
  const { byFolder, ungrouped } = useMemo(() => {
    const map = new Map<string, SessionInfo[]>();
    for (const folder of orderedFolders) map.set(folder.id, []);
    const loose: SessionInfo[] = [];
    for (const session of roots) {
      const bucket = session.folder_id
        ? map.get(session.folder_id)
        : undefined;
      if (bucket) bucket.push(session);
      else loose.push(session);
    }
    return { byFolder: map, ungrouped: loose };
  }, [roots, orderedFolders]);

  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(loadCollapsed);
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renamingFolderId, setRenamingFolderId] = useState<string | null>(null);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [dragOverKey, setDragOverKey] = useState<string | null>(null);
  const newFolderInputRef = useRef<HTMLInputElement>(null);
  const newFolderDoneRef = useRef(false);

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify(collapsed));
    } catch {
      // localStorage can be unavailable; collapsing is still session-local.
    }
  }, [collapsed]);

  useEffect(() => {
    if (!creatingFolder) return;
    newFolderDoneRef.current = false;
    const raf = requestAnimationFrame(() => newFolderInputRef.current?.focus());
    return () => cancelAnimationFrame(raf);
  }, [creatingFolder]);

  // Shared wall-clock tick (1min) so each row's relative "time since update"
  // stays live at minute granularity without spawning a timer per SessionItem.
  // Minute granularity keeps re-render frequency low (once per row per minute).
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60000);
    return () => clearInterval(t);
  }, []);

  // Wait for the WS to actually be usable before pulling the list. On startup
  // the socket is still handshaking when this panel mounts, so an immediate
  // `listSessions()` would race the connect and fail with "No connected WS".
  // Gating on `connected` (and re-firing on every reconnect) makes the
  // connecting → connected transition itself the trigger — no manual Retry
  // needed for the normal path. A genuine failure *after* connect still sets
  // `error` and surfaces the Retry affordance.
  useEffect(() => {
    if (connState === "connected") {
      listSessions();
    }
  }, [connState, listSessions]);

  const connecting = connState === "connecting" || connState === "reconnecting";

  const handleSessionClick = (sessionId: string) => {
    openSessionPanel(sessionId);
  };

  const toggle = (key: string) => {
    setCollapsed((current) => ({ ...current, [key]: !current[key] }));
  };

  // v1 allows duplicate names (no unique domain yet), but the user should know.
  const notifyDuplicateFolder = (name: string, exceptId?: string) => {
    if (
      folders.some(
        (folder) => folder.id !== exceptId && folder.name.trim() === name,
      )
    ) {
      useToastStore
        .getState()
        .showToast(i18n.t("session.duplicateFolderName"), "info");
    }
  };

  const isSessionDrag = (event: DragEvent) =>
    event.dataTransfer.types.includes(SESSION_DRAG_MIME);

  const handleDragOver = (event: DragEvent, key: string) => {
    if (!isSessionDrag(event)) return;
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "move";
    setDragOverKey(key);
  };

  const handleDragLeave = (event: DragEvent, key: string) => {
    const next = event.relatedTarget;
    if (next instanceof Node && event.currentTarget.contains(next)) return;
    setDragOverKey((current) => (current === key ? null : current));
  };

  const handleDrop = (event: DragEvent, folderId: string | null) => {
    event.preventDefault();
    event.stopPropagation();
    const id =
      event.dataTransfer.getData(SESSION_DRAG_MIME) ||
      event.dataTransfer.getData("text/plain");
    setDragOverKey(null);
    if (id && sessions.some((session) => session.id === id)) {
      moveSession(id, folderId);
    }
  };

  const commitNewFolder = () => {
    if (newFolderDoneRef.current) return;
    newFolderDoneRef.current = true;
    const name = newFolderName.trim();
    if (name) {
      notifyDuplicateFolder(name);
      createFolder(name);
    }
    setCreatingFolder(false);
    setNewFolderName("");
  };

  const cancelNewFolder = () => {
    newFolderDoneRef.current = true;
    setCreatingFolder(false);
    setNewFolderName("");
  };

  const renderSession = (session: SessionInfo, grouped = false) => (
    <SessionItem
      key={session.id}
      session={session}
      now={now}
      indentPx={grouped ? GROUPED_ROW_INDENT_PX : undefined}
      onOpen={handleSessionClick}
      onDelete={deleteSession}
      onContextMenu={(event: MouseEvent) => {
        event.preventDefault();
        event.stopPropagation();
        setMenu({
          kind: "session",
          id: session.id,
          x: event.clientX,
          y: event.clientY,
        });
      }}
      renaming={renamingSessionId === session.id}
      onStartRename={() => {
        setRenamingFolderId(null);
        setRenamingSessionId(session.id);
      }}
      onRename={(title) => {
        setRenamingSessionId(null);
        renameSession(session.id, title);
      }}
      onRenameCancel={() => setRenamingSessionId(null)}
    />
  );

  const menuItems = useMemo<FileTreeMenuItem[]>(() => {
    if (!menu) return [];
    if (menu.kind === "folder") {
      const folder = folders.find((item) => item.id === menu.id);
      if (!folder) return [];
      return [
        {
          id: "rename-folder",
          label: i18n.t("session.renameFolder"),
          onClick: () => {
            setCollapsed((current) => ({ ...current, [folder.id]: false }));
            setRenamingSessionId(null);
            setRenamingFolderId(folder.id);
          },
        },
        { id: "folder-separator", separator: true },
        {
          id: "delete-folder",
          label: i18n.t("session.deleteFolder"),
          danger: true,
          onClick: () => {
            if (
              window.confirm(
                i18n.t("session.deleteFolderConfirm", { name: folder.name }),
              )
            ) {
              deleteFolder(folder.id);
            }
          },
        },
      ];
    }
    const session = sessions.find((item) => item.id === menu.id);
    const items: FileTreeMenuItem[] = [
      {
        id: "rename-session",
        label: i18n.t("session.renameSession"),
        onClick: () => {
          setRenamingFolderId(null);
          setRenamingSessionId(menu.id);
        },
      },
    ];
    if (orderedFolders.length > 0) {
      items.push({ id: "session-separator", separator: true });
      for (const folder of orderedFolders) {
        items.push({
          id: `move-${folder.id}`,
          label: i18n.t("session.moveToFolderName", { name: folder.name }),
          disabled: session?.folder_id === folder.id,
          onClick: () => moveSession(menu.id, folder.id),
        });
      }
      if (session?.folder_id) {
        items.push({
          id: "move-ungrouped",
          label: i18n.t("session.removeFromFolder"),
          onClick: () => moveSession(menu.id, null),
        });
      }
    }
    return items;
  }, [
    menu,
    folders,
    orderedFolders,
    sessions,
    deleteFolder,
    moveSession,
    renameSession,
  ]);

  const renderBody = () => {
    if (
      roots.length === 0 &&
      orderedFolders.length === 0 &&
      !creatingFolder
    ) {
      // While the socket is establishing or recovering, show a connecting
      // state rather than a misleading error/retry.
      if (connecting && !error) {
        return (
          <p className="p-4 text-center text-xs text-(--_dk-text-disabled)">
            {i18n.t("common.connecting")}
          </p>
        );
      }
      if (error) {
        return (
          <div className="flex flex-col items-center gap-2 p-4 text-center">
            <p className="text-xs text-(--_dk-red-500)">{error}</p>
            <button
              type="button"
              onClick={() => listSessions()}
              className="btn-ghost"
            >
              {i18n.t("common.retry")}
            </button>
          </div>
        );
      }
      return (
        <p className="p-4 text-center text-xs text-(--_dk-text-disabled)">
          {loading
            ? i18n.t("session.loadingSessions")
            : i18n.t("session.noSavedSessions")}
        </p>
      );
    }
    return (
      <div>
        {creatingFolder && (
          <div className="flex h-8 items-center gap-1.5 px-2">
            <FolderPlus
              size={13}
              className="shrink-0 text-(--_dk-text-muted)"
            />
            <input
              ref={newFolderInputRef}
              value={newFolderName}
              onChange={(event) => setNewFolderName(event.target.value)}
              onClick={(event) => event.stopPropagation()}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  commitNewFolder();
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  cancelNewFolder();
                }
              }}
              onBlur={commitNewFolder}
              placeholder={i18n.t("session.folderName")}
              aria-label={i18n.t("session.folderName")}
              maxLength={120}
              className="min-w-0 flex-1 rounded border border-(--_dk-ix-ring) bg-(--_dk-editor) px-1 py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none"
            />
          </div>
        )}
        {orderedFolders.map((folder) => {
          const folderSessions = byFolder.get(folder.id) ?? [];
          const isCollapsed = Boolean(collapsed[folder.id]);
          return (
            <div key={folder.id}>
              <SessionFolderHeader
                folder={folder}
                count={folderSessions.length}
                collapsed={isCollapsed}
                renaming={renamingFolderId === folder.id}
                dragOver={dragOverKey === folder.id}
                onToggle={() => toggle(folder.id)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  setMenu({
                    kind: "folder",
                    id: folder.id,
                    x: event.clientX,
                    y: event.clientY,
                  });
                }}
                onRename={(name) => {
                  setRenamingFolderId(null);
                  notifyDuplicateFolder(name, folder.id);
                  renameFolder(folder.id, name);
                }}
                onRenameCancel={() => setRenamingFolderId(null)}
                onDragOver={(event) => handleDragOver(event, folder.id)}
                onDragLeave={(event) => handleDragLeave(event, folder.id)}
                onDrop={(event) => handleDrop(event, folder.id)}
              />
              {!isCollapsed &&
                (folderSessions.length > 0 ? (
                  folderSessions.map((session) => renderSession(session, true))
                ) : (
                  <p
                    className="py-1 pr-2 text-[10px] text-(--_dk-text-disabled)"
                    style={{ paddingLeft: FOLDER_LABEL_INSET_PX }}
                  >
                    {i18n.t("session.emptyFolder")}
                  </p>
                ))}
            </div>
          );
        })}
        {orderedFolders.length > 0 && (
          <div>
            <UngroupedHeader
              count={ungrouped.length}
              collapsed={Boolean(collapsed[UNGROUPED_KEY])}
              dragOver={dragOverKey === UNGROUPED_KEY}
              onToggle={() => toggle(UNGROUPED_KEY)}
              onDragOver={(event) => handleDragOver(event, UNGROUPED_KEY)}
              onDragLeave={(event) => handleDragLeave(event, UNGROUPED_KEY)}
              onDrop={(event) => handleDrop(event, null)}
            />
            {!collapsed[UNGROUPED_KEY] &&
              ungrouped.map((session) => renderSession(session, true))}
          </div>
        )}
        {orderedFolders.length === 0 &&
          roots.map((session) => renderSession(session))}
      </div>
    );
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-3 py-2">
        <div className="flex items-center gap-1">
          <button type="button" onClick={newSession} className="btn-ghost btn-sm">
            {i18n.t("common.new")}
          </button>
          <button
            type="button"
            onClick={() => {
              setCreatingFolder(true);
              setRenamingFolderId(null);
            }}
            aria-label={i18n.t("session.newFolder")}
            title={i18n.t("session.newFolder")}
            className="flex h-6 w-6 items-center justify-center rounded-md text-(--_dk-text-secondary) transition-colors hover:bg-(--_dk-ix-bg-hover)"
          >
            <FolderPlus size={13} />
          </button>
        </div>
        <div className="flex min-w-0 items-center gap-2">
          {error && connState === "connected" && (
            <button
              type="button"
              onClick={() => listSessions()}
              className={`btn-ghost text-(--_dk-ix-danger-fg)`}
              title={error}
            >
              {i18n.t("common.retry")}
            </button>
          )}
          {(loading || connecting) && (
            <span
              className="h-3 w-3 shrink-0 animate-spin rounded-full border border-(--_dk-text-muted) border-t-transparent"
              title={
                connecting
                  ? i18n.t("common.connecting")
                  : i18n.t("session.loadingSessions")
              }
            />
          )}
          {/* Workspace spend sits at the far right, where a quiet read-out
              belongs. Same popover shell as the per-session ctx ring, so the two
              usage surfaces read as one idea. Reads only on open. */}
          <Popover
            width={240}
            trigger={({ open, toggle: togglePopover }) => (
              <button
                type="button"
                onClick={togglePopover}
                aria-expanded={open}
                aria-label={i18n.t("session.workspaceUsage")}
                title={i18n.t("session.workspaceUsage")}
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-(--_dk-text-secondary) transition-colors hover:bg-(--_dk-ix-bg-hover)"
              >
                <Info size={13} />
              </button>
            )}
          >
            <WorkspaceUsage />
          </Popover>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">{renderBody()}</div>
      {menu && menuItems.length > 0 && (
        <FileTreeContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
