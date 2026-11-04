import {
  useEffect,
  useMemo,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import { Info, Robot, StackSimple } from "@phosphor-icons/react";

import type { BotInfo, SessionFolder, SessionInfo } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { useBotStore } from "../stores/botStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { useUnreadStore } from "../stores/unreadStore";
import { BotDialog, type BotDialogMode } from "./BotDialog";
import { BotItem } from "./BotItem";
import { SessionItem } from "./SessionItem";
import { SessionFolderHeader } from "./SessionFolderHeader";
import { ContextMenu, type ContextMenuItem } from "./ui/ContextMenu";
import { useConfirmDialog } from "./ui/useConfirmDialog";
import { ListRow } from "./ui/ListRow";
import { Popover } from "./ui/Popover";
import { WorkspaceUsage } from "./WorkspaceUsage";
import { folderScope, type FolderScope } from "../lib/botFolders";
import { reorderFolderIds } from "../lib/sessionFolders";
import { openSessionPanel } from "../lib/sessionPanelNav";
import { watchActiveCenterSessionIds } from "../dockview/workbench/queries";
import i18n from "../i18n";

const COLLAPSED_KEY = "litecode.sessionFolders.collapsed";
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

function SectionHeader({
  icon,
  label,
  count,
}: {
  icon: ReactNode;
  label: string;
  count: number;
}) {
  return (
    <ListRow as="div" size="section" className="text-(--_dk-text-secondary)">
      {icon}
      <span className="min-w-0 flex-1 truncate text-[10px] font-medium uppercase tracking-wide text-(--_dk-text-muted)">
        {label}
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
  const botFolders = useMemo(
    () => orderedFolders.filter((folder) => folderScope(folder.id) === "bot"),
    [orderedFolders],
  );
  const sessionFolders = useMemo(
    () => orderedFolders.filter((folder) => folderScope(folder.id) === "session"),
    [orderedFolders],
  );
  const { botByFolder, botUngrouped, sessionByFolder, sessionUngrouped } =
    useMemo(() => {
      const botMap = new Map<string, BotInfo[]>();
      const sessionMap = new Map<string, SessionInfo[]>();
      for (const folder of botFolders) botMap.set(folder.id, []);
      for (const folder of sessionFolders) sessionMap.set(folder.id, []);
      const looseBots: BotInfo[] = [];
      for (const bot of orderedBots) {
        const folder = bot.folder_id;
        const bucket =
          folder && folderScope(folder) === "bot"
            ? botMap.get(folder)
            : undefined;
        if (bucket) bucket.push(bot);
        else looseBots.push(bot);
      }
      const looseSessions: SessionInfo[] = [];
      for (const session of roots) {
        const folder = session.folder_id;
        const bucket =
          folder && folderScope(folder) === "session"
            ? sessionMap.get(folder)
            : undefined;
        if (bucket) bucket.push(session);
        else looseSessions.push(session);
      }
      return {
        botByFolder: botMap,
        botUngrouped: looseBots,
        sessionByFolder: sessionMap,
        sessionUngrouped: looseSessions,
      };
    }, [roots, orderedBots, botFolders, sessionFolders]);

  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(loadCollapsed);
  // Sessions currently shown by the agent-area groups. Tracked from the
  // workbench (not panel isActive) so the marker survives clicking the list.
  const [activeSessionIds, setActiveSessionIds] = useState<readonly string[]>([]);
  const unread = useUnreadStore((s) => s.unread);
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renamingBotId, setRenamingBotId] = useState<string | null>(null);
  const [renamingFolderId, setRenamingFolderId] = useState<string | null>(null);
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

  // v1 allows duplicate names, but the user should know within one domain.
  const notifyDuplicateFolder = (name: string, exceptId?: string) => {
    const scope = exceptId ? folderScope(exceptId) : undefined;
    if (
      folders.some(
        (folder) =>
          folder.id !== exceptId &&
          folder.name.trim() === name &&
          (!scope || folderScope(folder.id) === scope),
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
    if (folderScope(sourceId) !== folderScope(targetId)) return;
    const next = reorderFolderIds(
      orderedFolders.map((folder) => folder.id),
      sourceId,
      targetId,
      position,
    );
    if (next) reorderFolders(next);
  };

  const moveFolder = (id: string, delta: -1 | 1) => {
    const scope = folderScope(id);
    const peers = orderedFolders.filter(
      (folder) => folderScope(folder.id) === scope,
    );
    const index = peers.findIndex((folder) => folder.id === id);
    const neighbor = peers[index + delta];
    if (!neighbor) return;
    reorderFolder(id, neighbor.id, delta < 0 ? "before" : "after");
  };

  const focusGroup = (folderId: string) => {
    setCollapsed((current) => ({ ...current, [folderId]: false }));
    setRenamingSessionId(null);
    setRenamingBotId(null);
    setRenamingFolderId(folderId);
  };

  const moveRow = async (
    scope: FolderScope,
    id: string,
    folderId: string,
  ) => {
    if (scope === "bot") {
      await moveBot(id, folderId);
    } else {
      moveSession(id, folderId);
    }
  };

  /** New group from a row's context menu: create it with that row inside. */
  const startGroup = async (scope: FolderScope, id: string) => {
    const folderId = await createFolder(i18n.t("session.newGroup"), scope);
    if (!folderId) return;
    await moveRow(scope, id, folderId);
    focusGroup(folderId);
  };

  /** Row-on-row drop: join the target's group, or seed a new one with both. */
  const groupRows = async (
    scope: FolderScope,
    sourceId: string,
    targetId: string,
  ) => {
    if (sourceId === targetId) return;
    const rows = scope === "bot" ? bots : sessions;
    const byId = new Map(rows.map((row) => [row.id, row]));
    const targetFolder = byId.get(targetId)?.folder_id ?? null;
    const sourceFolder = byId.get(sourceId)?.folder_id ?? null;
    const usable = (folder: string | null) =>
      folder && folderScope(folder) === scope ? folder : null;
    const targetGroup = usable(targetFolder);
    const sourceGroup = usable(sourceFolder);
    if (targetGroup) {
      if (sourceGroup !== targetGroup) {
        await moveRow(scope, sourceId, targetGroup);
      }
      return;
    }
    if (sourceGroup) {
      await moveRow(scope, targetId, sourceGroup);
      return;
    }
    const folderId = await createFolder(i18n.t("session.newGroup"), scope);
    if (!folderId) return;
    await moveRow(scope, sourceId, folderId);
    await moveRow(scope, targetId, folderId);
    focusGroup(folderId);
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
      onDropSession={(sourceId) =>
        void groupRows("session", sourceId, session.id)
      }
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
      onDropBot={(sourceId) => void groupRows("bot", sourceId, bot.id)}
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

  const renderBotEntries = (entries: BotInfo[], grouped = false) =>
    entries.map((bot) => renderBot(bot, grouped));

  const renderSessionEntries = (entries: SessionInfo[], grouped = false) =>
    entries.map((session) => renderSession(session, grouped));

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
        {
          id: "group-bot",
          label: i18n.t("session.newGroup"),
          restoreFocus: false,
          onClick: () => void startGroup("bot", bot.id),
        },
      ];
      const botFoldersForMenu = orderedFolders.filter(
        (folder) => folderScope(folder.id) === "bot",
      );
      if (botFoldersForMenu.length > 0) {
        items.push({ id: "bot-separator", separator: true });
        for (const folder of botFoldersForMenu) {
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
      {
        id: "group-session",
        label: i18n.t("session.newGroup"),
        restoreFocus: false,
        onClick: () => void startGroup("session", menu.id),
      },
    ];
    const sessionFoldersForMenu = orderedFolders.filter(
      (folder) => folderScope(folder.id) === "session",
    );
    if (sessionFoldersForMenu.length > 0) {
      items.push({ id: "session-separator", separator: true });
      for (const folder of sessionFoldersForMenu) {
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
    startGroup,
  ]);

  const renderFolder = (folder: SessionFolder, scope: FolderScope) => {
    const entries =
      scope === "bot"
        ? (botByFolder.get(folder.id) ?? [])
        : (sessionByFolder.get(folder.id) ?? []);
    const isCollapsed = Boolean(collapsed[folder.id]);
    return (
      <div key={folder.id}>
        <SessionFolderHeader
          folder={folder}
          count={entries.length}
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
          (entries.length > 0 ? (
            scope === "bot" ? (
              renderBotEntries(entries as BotInfo[], true)
            ) : (
              renderSessionEntries(entries as SessionInfo[], true)
            )
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
  };

  const renderBody = () => {
    if (roots.length === 0 && orderedFolders.length === 0 && bots.length === 0) {
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
    const hasBots = orderedBots.length > 0 || botFolders.length > 0;
    const hasSessions = roots.length > 0 || sessionFolders.length > 0;
    return (
      <div>
        {hasBots && (
          <div>
            <SectionHeader
              icon={
                <Robot
                  size={13}
                  className="shrink-0 text-(--_dk-text-muted)"
                  aria-hidden
                />
              }
              label={i18n.t("bot.title")}
              count={orderedBots.length}
            />
            {botFolders.map((folder) => renderFolder(folder, "bot"))}
            {renderBotEntries(botUngrouped)}
          </div>
        )}
        {hasSessions && (
          <div
            className={
              hasBots ? "mt-2 border-t border-(--_dk-line) pt-2" : undefined
            }
          >
            <SectionHeader
              icon={
                <StackSimple
                  size={13}
                  className="shrink-0 text-(--_dk-text-muted)"
                  aria-hidden
                />
              }
              label={i18n.t("session.sectionTitle")}
              count={roots.length}
            />
            {sessionFolders.map((folder) => renderFolder(folder, "session"))}
            {renderSessionEntries(sessionUngrouped)}
          </div>
        )}
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
