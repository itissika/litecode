import {
  Fragment,
  useEffect,
  useMemo,
  useState,
  type MouseEvent,
} from "react";
import {
  CaretDown,
  CaretRight,
  FolderPlus,
  Info,
  Robot,
  StackSimple,
} from "@phosphor-icons/react";

import type { BotInfo, SessionInfo } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { useBotStore } from "../stores/botStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { useUnreadStore } from "../stores/unreadStore";
import { BotDialog, type BotDialogMode } from "./BotDialog";
import { BotItem } from "./BotItem";
import { SessionItem } from "./SessionItem";
import { SessionFolderHeader } from "./SessionFolderHeader";
import {
  BOT_DRAG_MIME,
  parseBotDrag,
  parseSessionDrag,
  SESSION_DRAG_MIME,
  type BotDragPayload,
  type SessionDragPayload,
} from "./dnd/sessionDnd";
import { useDropTarget } from "./dnd/useDropTarget";
import { ContextMenu, type ContextMenuItem } from "./ui/ContextMenu";
import { useConfirmDialog } from "./ui/useConfirmDialog";
import { InlineRename } from "./ui/InlineRename";
import { ListRow } from "./ui/ListRow";
import { Popover } from "./ui/Popover";
import { WorkspaceUsage } from "./WorkspaceUsage";
import {
  moveFolderId,
  reorderFolderIds,
} from "../lib/sessionFolders";
import { openSessionPanel } from "../lib/sessionPanelNav";
import { watchActiveCenterSessionIds } from "../dockview/workbench/queries";
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
  | { kind: "bot"; id: string; x: number; y: number }
  | { kind: "folder"; id: string; x: number; y: number };

type SidebarEntry =
  | { kind: "session"; session: SessionInfo }
  | { kind: "bot"; bot: BotInfo; session?: SessionInfo };

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
  onToggle,
  onDropSession,
  onDropBot,
}: {
  count: number;
  collapsed: boolean;
  onToggle: () => void;
  onDropSession: (sessionId: string) => void;
  onDropBot: (botId: string) => void;
}) {
  const drop = useDropTarget<SessionDragPayload | BotDragPayload>([
    {
      mime: SESSION_DRAG_MIME,
      parse: parseSessionDrag,
      onDrop: (payload) => onDropSession(payload.id),
    },
    {
      mime: BOT_DRAG_MIME,
      parse: parseBotDrag,
      onDrop: (payload) => onDropBot(payload.id),
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
      }: ${i18n.t("session.ungrouped")}`}
      {...drop.handlers}
      onClick={onToggle}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onToggle();
        }
      }}
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
      <StackSimple size={13} className="shrink-0 text-(--_dk-text-muted)" />
      <span className="min-w-0 flex-1 truncate">
        {i18n.t("session.ungrouped")}
      </span>
      <span className="ml-auto shrink-0 text-[10px] text-(--_dk-text-disabled)">
        {count}
      </span>
    </ListRow>
  );
}

export function SessionList() {
  const sessions = useSessionStore((s) => s.sessions);
  const folders = useSessionStore((s) => s.folders);
  const bots = useBotStore((s) => s.bots);
  const botsError = useBotStore((s) => s.botsError);
  const loading = useSessionStore((s) => s.sessionsLoading);
  const error = useSessionStore((s) => s.sessionListError);
  const deleteSession = useSessionStore((s) => s.deleteSession);
  const newSession = useSessionStore((s) => s.newSession);
  const listSessions = useSessionStore((s) => s.listSessions);
  const renameSession = useSessionStore((s) => s.renameSession);
  const moveSession = useSessionStore((s) => s.moveSession);
  const listBots = useBotStore((s) => s.listBots);
  const openBot = useBotStore((s) => s.openBot);
  const updateBot = useBotStore((s) => s.updateBot);
  const deleteBot = useBotStore((s) => s.deleteBot);
  const moveBot = useBotStore((s) => s.moveBot);
  const createFolder = useSessionStore((s) => s.createFolder);
  const renameFolder = useSessionStore((s) => s.renameFolder);
  const deleteFolder = useSessionStore((s) => s.deleteFolder);
  const reorderFolders = useSessionStore((s) => s.reorderFolders);
  const connState = useConnectionStore((s) => s.state);
  const { confirm, dialog } = useConfirmDialog();

  // The server also lists subagent CHILD sessions (the roster needs them to
  // label a child that was never subscribed), so filter the sidebar to roots.
  // That is the only rule here: emptiness / visibility is decided by the
  // server's list projection (`SessionListRow::is_listed`), never re-derived
  // in the sidebar.
  // A bot's default session is an ordinary session on the server, but the
  // sidebar gives the bot one identity row instead of two entries.
  const sessionsById = useMemo(
    () => new Map(sessions.map((session) => [session.id, session])),
    [sessions],
  );
  const orderedBots = useMemo(
    () =>
      [...bots].sort(
        (left, right) =>
          (left.sort_order ?? 0) - (right.sort_order ?? 0) ||
          left.created_at - right.created_at ||
          left.name.localeCompare(right.name),
      ),
    [bots],
  );
  const botSessionIds = useMemo(
    () => new Set(bots.map((bot) => bot.default_session_id)),
    [bots],
  );
  const roots = useMemo(
    () =>
      sessions.filter(
        (session) =>
          !session.parent_session_id && !botSessionIds.has(session.id),
      ),
    [sessions, botSessionIds],
  );
  const orderedFolders = useMemo(
    () => [...folders].sort((a, b) => a.sort_order - b.sort_order),
    [folders],
  );
  const { byFolder, ungrouped } = useMemo(() => {
    const map = new Map<string, SidebarEntry[]>();
    for (const folder of orderedFolders) map.set(folder.id, []);
    const loose: SidebarEntry[] = [];
    // Bots lead each bucket; a divider separates them from sessions. Both use
    // the same session folder ids.
    for (const bot of orderedBots) {
      const entry: SidebarEntry = {
        kind: "bot",
        bot,
        session: sessionsById.get(bot.default_session_id),
      };
      const bucket = bot.folder_id ? map.get(bot.folder_id) : undefined;
      if (bucket) bucket.push(entry);
      else loose.push(entry);
    }
    for (const session of roots) {
      const entry: SidebarEntry = { kind: "session", session };
      const bucket = session.folder_id
        ? map.get(session.folder_id)
        : undefined;
      if (bucket) bucket.push(entry);
      else loose.push(entry);
    }
    return { byFolder: map, ungrouped: loose };
  }, [roots, orderedBots, orderedFolders, sessionsById]);

  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(loadCollapsed);
  // Sessions currently shown by the agent-area groups. Tracked from the
  // workbench (not panel isActive) so the marker survives clicking the list.
  const [activeSessionIds, setActiveSessionIds] = useState<readonly string[]>([]);
  const unread = useUnreadStore((s) => s.unread);
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renamingBotId, setRenamingBotId] = useState<string | null>(null);
  const [renamingFolderId, setRenamingFolderId] = useState<string | null>(null);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [botDialog, setBotDialog] = useState<BotDialogMode | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify(collapsed));
    } catch {
      // localStorage can be unavailable; collapsing is still session-local.
    }
  }, [collapsed]);

  useEffect(() => watchActiveCenterSessionIds(setActiveSessionIds), []);

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
      listBots();
    }
  }, [connState, listSessions, listBots]);

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

  const reorderFolder = (
    sourceId: string,
    targetId: string,
    position: "before" | "after",
  ) => {
    const next = reorderFolderIds(
      orderedFolders.map((folder) => folder.id),
      sourceId,
      targetId,
      position,
    );
    if (next) reorderFolders(next);
  };

  const moveFolder = (id: string, delta: -1 | 1) => {
    const next = moveFolderId(
      orderedFolders.map((folder) => folder.id),
      id,
      delta,
    );
    if (next) reorderFolders(next);
  };

  const botSession = (bot: BotInfo) =>
    sessionsById.get(bot.default_session_id);
  const botBusy = (bot: BotInfo) => {
    const session = botSession(bot);
    return Boolean(
      session?.running ||
        session?.status === "running_with_subagent" ||
        session?.status === "stopping",
    );
  };
  const openBotById = async (id: string) => {
    const sessionId = await openBot(id);
    if (sessionId) openSessionPanel(sessionId);
  };
  const startBotRename = (id: string) => {
    setRenamingSessionId(null);
    setRenamingFolderId(null);
    setRenamingBotId(id);
  };
  const commitBotRename = async (bot: BotInfo, name: string) => {
    setRenamingBotId(null);
    const trimmed = name.trim();
    if (!trimmed || trimmed === bot.name) return;
    await updateBot(bot.id, { name: trimmed });
  };
  const requestDeleteBot = (bot: BotInfo) => {
    if (botBusy(bot)) {
      useToastStore
        .getState()
        .showToast(i18n.t("bot.cannotDeleteBusy"), "info");
      return;
    }
    void confirm({
      title: i18n.t("bot.deleteTitle"),
      body: i18n.t("bot.deleteConfirm", { name: bot.name }),
      danger: true,
      confirmLabel: i18n.t("common.delete"),
      cancelLabel: i18n.t("common.cancel"),
    }).then((ok) => {
      if (ok) void deleteBot(bot.id);
    });
  };

  const renderSession = (session: SessionInfo, grouped = false) => (
    <SessionItem
      key={session.id}
      session={session}
      now={now}
      indentPx={grouped ? GROUPED_ROW_INDENT_PX : undefined}
      focused={activeSessionIds.includes(session.id)}
      unread={unread[session.id] === true}
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
        setRenamingBotId(null);
        setRenamingSessionId(session.id);
      }}
      onRename={(title) => {
        setRenamingSessionId(null);
        renameSession(session.id, title);
      }}
      onRenameCancel={() => setRenamingSessionId(null)}
    />
  );

  const renderBot = (bot: BotInfo, grouped = false) => (
    <BotItem
      key={bot.id}
      bot={bot}
      session={botSession(bot)}
      now={now}
      indentPx={grouped ? GROUPED_ROW_INDENT_PX : undefined}
      focused={activeSessionIds.includes(bot.default_session_id)}
      unread={unread[bot.default_session_id] === true}
      onOpen={(id) => void openBotById(id)}
      onDelete={requestDeleteBot}
      onContextMenu={(event: MouseEvent) => {
        event.preventDefault();
        event.stopPropagation();
        setMenu({
          kind: "bot",
          id: bot.id,
          x: event.clientX,
          y: event.clientY,
        });
      }}
      renaming={renamingBotId === bot.id}
      onStartRename={() => startBotRename(bot.id)}
      onRename={(name) => void commitBotRename(bot, name)}
      onRenameCancel={() => setRenamingBotId(null)}
    />
  );

  const renderEntry = (entry: SidebarEntry, grouped = false) =>
    entry.kind === "session"
      ? renderSession(entry.session, grouped)
      : renderBot(entry.bot, grouped);

  const renderEntries = (entries: SidebarEntry[], grouped = false) => {
    const firstSession = entries.findIndex((entry) => entry.kind === "session");
    return entries.map((entry, index) => (
      <Fragment
        key={entry.kind === "session" ? entry.session.id : entry.bot.id}
      >
        {index === firstSession && firstSession > 0 ? (
          <div
            aria-hidden
            className="mx-3 my-1 border-t border-(--_dk-line)"
          />
        ) : null}
        {renderEntry(entry, grouped)}
      </Fragment>
    ));
  };

  const menuItems = useMemo<ContextMenuItem[]>(() => {
    if (!menu) return [];
    if (menu.kind === "folder") {
      const folder = folders.find((item) => item.id === menu.id);
      if (!folder) return [];
      return [
        {
          id: "rename-folder",
          label: i18n.t("session.renameFolder"),
          restoreFocus: false,
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
          restoreFocus: false,
          onClick: () => {
            void confirm({
              title: i18n.t("session.deleteFolder"),
              body: i18n.t("session.deleteFolderConfirm", { name: folder.name }),
              danger: true,
              confirmLabel: i18n.t("common.delete"),
              cancelLabel: i18n.t("common.cancel"),
            }).then((ok) => {
              if (ok) deleteFolder(folder.id);
            });
          },
        },
      ];
    }
    if (menu.kind === "bot") {
      const bot = bots.find((item) => item.id === menu.id);
      if (!bot) return [];
      const items: ContextMenuItem[] = [
        {
          id: "edit-bot",
          label: i18n.t("bot.edit"),
          restoreFocus: false,
          onClick: () => setBotDialog({ kind: "edit", bot }),
        },
        {
          id: "rename-bot",
          label: i18n.t("bot.rename"),
          restoreFocus: false,
          onClick: () => startBotRename(bot.id),
        },
      ];
      if (orderedFolders.length > 0) {
        items.push({ id: "bot-separator", separator: true });
        for (const folder of orderedFolders) {
          items.push({
            id: `move-bot-${folder.id}`,
            label: i18n.t("session.moveToFolderName", { name: folder.name }),
            disabled: bot.folder_id === folder.id,
            onClick: () => void moveBot(bot.id, folder.id),
          });
        }
        if (bot.folder_id) {
          items.push({
            id: "move-bot-ungrouped",
            label: i18n.t("session.removeFromFolder"),
            onClick: () => void moveBot(bot.id, null),
          });
        }
      }
      items.push({ id: "bot-danger-separator", separator: true });
      items.push({
        id: "delete-bot",
        label: i18n.t("bot.delete"),
        danger: true,
        disabled: botBusy(bot),
        onClick: () => requestDeleteBot(bot),
      });
      return items;
    }
    const session = sessions.find((item) => item.id === menu.id);
    const items: ContextMenuItem[] = [
      {
        id: "rename-session",
        label: i18n.t("session.renameSession"),
        restoreFocus: false,
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
    bots,
    deleteFolder,
    moveSession,
    renameSession,
    moveBot,
    requestDeleteBot,
  ]);

  const renderBody = () => {
    if (
      roots.length === 0 &&
      orderedFolders.length === 0 &&
      bots.length === 0 &&
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
            <InlineRename
              initialValue=""
              ariaLabel={i18n.t("session.folderName")}
              placeholder={i18n.t("session.folderName")}
              maxLength={120}
              onCommit={(name) => {
                setCreatingFolder(false);
                if (!name) return;
                notifyDuplicateFolder(name);
                createFolder(name);
              }}
              onCancel={() => setCreatingFolder(false)}
              className="min-w-0 flex-1 rounded border border-(--_dk-ix-ring) bg-(--_dk-editor) px-1 py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none"
            />
          </div>
        )}
        {orderedFolders.map((folder) => {
          const folderEntries = byFolder.get(folder.id) ?? [];
          const isCollapsed = Boolean(collapsed[folder.id]);
          return (
            <div key={folder.id}>
              <SessionFolderHeader
                folder={folder}
                count={folderEntries.length}
                collapsed={isCollapsed}
                renaming={renamingFolderId === folder.id}
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
                onDropSession={(sessionId) => {
                  if (sessions.some((session) => session.id === sessionId)) {
                    moveSession(sessionId, folder.id);
                  }
                }}
                onDropBot={(botId) => void moveBot(botId, folder.id)}
                onReorderFolder={reorderFolder}
                onMoveFolder={moveFolder}
              />
              {!isCollapsed &&
                (folderEntries.length > 0 ? (
                  renderEntries(folderEntries, true)
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
              onToggle={() => toggle(UNGROUPED_KEY)}
              onDropSession={(sessionId) => {
                if (sessions.some((session) => session.id === sessionId)) {
                  moveSession(sessionId, null);
                }
              }}
              onDropBot={(botId) => void moveBot(botId, null)}
            />
            {!collapsed[UNGROUPED_KEY] && renderEntries(ungrouped, true)}
          </div>
        )}
        {orderedFolders.length === 0 && renderEntries(ungrouped)}
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
            onClick={() => setBotDialog({ kind: "create" })}
            aria-label={i18n.t("bot.new")}
            title={i18n.t("bot.new")}
            className="flex h-6 w-6 items-center justify-center rounded-md text-(--_dk-text-secondary) transition-colors hover:bg-(--_dk-ix-bg-hover)"
          >
            <Robot size={13} />
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
            width="auto"
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

      <div className="flex-1 overflow-y-auto">
        {botsError && bots.length === 0 && (
          <button
            type="button"
            onClick={listBots}
            title={botsError}
            className="block w-full px-3 py-2 text-left text-[10px] text-(--_dk-ix-danger-fg)"
          >
            {i18n.t("bot.loadFailed")} · {i18n.t("bot.retry")}
          </button>
        )}
        {renderBody()}
      </div>
      {menu && menuItems.length > 0 && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems}
          onClose={() => setMenu(null)}
        />
      )}
      {dialog}
      {botDialog && (
        <BotDialog
          mode={botDialog}
          onClose={() => setBotDialog(null)}
          onCreated={(bot) => void openBotById(bot.id)}
        />
      )}
    </div>
  );
}
