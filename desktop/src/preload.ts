import { contextBridge, ipcRenderer, webUtils } from "electron";

import type {
  BROWSER_AGENT_ACTIVITY_CHANNEL as BrowserAgentActivityChannel,
  BROWSER_CLOSE_CHANNEL as BrowserCloseChannel,
  BROWSER_OPEN_CHANNEL as BrowserOpenChannel,
  BROWSER_STATE_CHANNEL as BrowserStateChannel,
  BrowserAgentActivityEvent,
  BrowserCloseRequest,
  BrowserOpenRequest,
  COMPONENT_PROGRESS_CHANNEL as ComponentProgressChannel,
  REMOTE_DISCONNECTED_CHANNEL as RemoteDisconnectedChannel,
  REMOTE_PROGRESS_CHANNEL as RemoteProgressChannel,
  BrowserBounds,
  BrowserState,
  ComponentProgressEvent,
  ComponentStatus,
  EnsureComponentResult,
  LitecodeDesktopBridge,
  RecentWorkspace,
  RemoteDisconnectedEvent,
  RemoteHistoryItem,
  RemoteProgressEvent,
  StartRemoteSessionInput,
  TURN_NOTIFICATION_CLICK_CHANNEL as TurnNotificationClickChannel,
  TurnNotificationClickEvent,
  TurnNotificationPayload,
} from "./bridge-contract";

/*
 * Sandboxed preload may only `require` Electron/Node builtins, not local files,
 * so channel names stay literal here but are typed against the contract
 * constants: any drift fails to compile.
 */
const REMOTE_PROGRESS_CHANNEL: typeof RemoteProgressChannel = "litecode:remote-progress";
const REMOTE_DISCONNECTED_CHANNEL: typeof RemoteDisconnectedChannel = "litecode:remote-disconnected";
const COMPONENT_PROGRESS_CHANNEL: typeof ComponentProgressChannel = "litecode:component-progress";
const BROWSER_STATE_CHANNEL: typeof BrowserStateChannel = "litecode:browser-state";
const BROWSER_OPEN_CHANNEL: typeof BrowserOpenChannel = "litecode:browser-open";
const BROWSER_CLOSE_CHANNEL: typeof BrowserCloseChannel = "litecode:browser-close";
const BROWSER_AGENT_ACTIVITY_CHANNEL: typeof BrowserAgentActivityChannel =
  "litecode:browser-agent-activity";
const TURN_NOTIFICATION_CLICK_CHANNEL: typeof TurnNotificationClickChannel =
  "litecode:turn-notification-click";

const bridge = {
  getAuthToken: (): string | undefined => {
    return ipcRenderer.sendSync("litecode:get-auth-token") as string | undefined;
  },
  getPathForFile: (file: File): string => {
    try {
      const value = webUtils.getPathForFile(file);
      return typeof value === "string" ? value : "";
    } catch {
      return "";
    }
  },
  getSessionMode: (): "local" | "remote" => {
    const mode = ipcRenderer.sendSync("litecode:get-session-mode") as string | undefined;
    return mode === "remote" ? "remote" : "local";
  },
  loadLayout: (): string | null => {
    const raw = ipcRenderer.sendSync("litecode:load-layout") as
      | string
      | undefined;
    return typeof raw === "string" && raw.length > 0 ? raw : null;
  },
  saveLayout: (payload: string): void => {
    void ipcRenderer.invoke("litecode:save-layout", payload);
  },
  pickFolder: async (): Promise<string | null> => {
    return (await ipcRenderer.invoke("litecode:pick-folder")) as string | null;
  },
  listRecents: async (): Promise<RecentWorkspace[]> => {
    return (await ipcRenderer.invoke("litecode:list-recents")) as RecentWorkspace[];
  },
  setRecentPinned: async (workspacePath: string, pinned: boolean): Promise<RecentWorkspace[]> => {
    return (await ipcRenderer.invoke(
      "litecode:set-recent-pinned",
      workspacePath,
      pinned,
    )) as RecentWorkspace[];
  },
  removeRecent: async (workspacePath: string): Promise<RecentWorkspace[]> => {
    return (await ipcRenderer.invoke("litecode:remove-recent", workspacePath)) as RecentWorkspace[];
  },
  listRemoteHistory: async (): Promise<RemoteHistoryItem[]> => {
    return (await ipcRenderer.invoke("litecode:list-remote-history")) as RemoteHistoryItem[];
  },
  setRemoteHistoryPinned: async (id: string, pinned: boolean): Promise<void> => {
    await ipcRenderer.invoke("litecode:set-remote-history-pinned", id, pinned);
  },
  removeSshTarget: async (id: string): Promise<void> => {
    await ipcRenderer.invoke("litecode:remove-ssh-target", id);
  },
  startRemoteSession: async (input: StartRemoteSessionInput): Promise<{ sessionId: string; home: string; label: string }> => {
    return (await ipcRenderer.invoke("litecode:start-remote-session", input)) as {
      sessionId: string;
      home: string;
      label: string;
    };
  },
  listPendingRemoteDirs: async (
    sessionId: string,
    remotePath = ".",
  ): Promise<{ path: string; home: string; entries: Array<{ name: string }> }> => {
    return (await ipcRenderer.invoke("litecode:list-pending-remote-dirs", {
      sessionId,
      path: remotePath,
    })) as { path: string; home: string; entries: Array<{ name: string }> };
  },
  completeRemoteSession: async (
    sessionId: string,
    workspace: string,
  ): Promise<{ token: string; baseUrl: string; workspace: string; label: string }> => {
    return (await ipcRenderer.invoke("litecode:complete-remote-session", {
      sessionId,
      workspace,
    })) as { token: string; baseUrl: string; workspace: string; label: string };
  },
  enterRemoteWorkbench: async (sessionId: string): Promise<{ ok: boolean; mode: "remote" }> => {
    return (await ipcRenderer.invoke("litecode:enter-remote-workbench", sessionId)) as {
      ok: boolean;
      mode: "remote";
    };
  },
  cancelRemoteSession: async (sessionId: string): Promise<void> => {
    await ipcRenderer.invoke("litecode:cancel-remote-session", sessionId);
  },
  reconnectRemote: async (id: string): Promise<{ ok: boolean; mode: "remote" }> => {
    return (await ipcRenderer.invoke("litecode:reconnect-remote", id)) as {
      ok: boolean;
      mode: "remote";
    };
  },
  onRemoteProgress: (handler: (event: RemoteProgressEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: RemoteProgressEvent) => {
      handler(payload);
    };
    ipcRenderer.on(REMOTE_PROGRESS_CHANNEL, listener);
    return () => ipcRenderer.removeListener(REMOTE_PROGRESS_CHANNEL, listener);
  },
  onRemoteDisconnected: (handler: (event: RemoteDisconnectedEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: RemoteDisconnectedEvent) => {
      handler(payload);
    };
    ipcRenderer.on(REMOTE_DISCONNECTED_CHANNEL, listener);
    return () => ipcRenderer.removeListener(REMOTE_DISCONNECTED_CHANNEL, listener);
  },
  getComponentStatus: async (id: string): Promise<ComponentStatus> => {
    return (await ipcRenderer.invoke("litecode:component-status", id)) as ComponentStatus;
  },
  ensureComponent: async (
    id: string,
  ): Promise<EnsureComponentResult> => {
    return (await ipcRenderer.invoke("litecode:ensure-component", id)) as EnsureComponentResult;
  },
  onComponentProgress: (handler: (event: ComponentProgressEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: ComponentProgressEvent) => {
      handler(payload);
    };
    ipcRenderer.on(COMPONENT_PROGRESS_CHANNEL, listener);
    return () => ipcRenderer.removeListener(COMPONENT_PROGRESS_CHANNEL, listener);
  },
  notifyTurnEnd: (payload: TurnNotificationPayload): void => {
    ipcRenderer.send("litecode:notify-turn-end", payload);
  },
  onTurnNotificationClick: (
    handler: (event: TurnNotificationClickEvent) => void,
  ): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: TurnNotificationClickEvent) => {
      handler(payload);
    };
    ipcRenderer.on(TURN_NOTIFICATION_CLICK_CHANNEL, listener);
    return () => ipcRenderer.removeListener(TURN_NOTIFICATION_CLICK_CHANNEL, listener);
  },
  notifyWorkspace: async (workspacePath: string | null): Promise<void> => {
    await ipcRenderer.invoke("litecode:notify-workspace", workspacePath);
  },
  openWorkspace: async (
    workspacePath: string,
  ): Promise<{ ok: boolean; focused?: boolean; project: string }> => {
    return (await ipcRenderer.invoke("litecode:open-workspace", workspacePath)) as {
      ok: boolean;
      focused?: boolean;
      project: string;
    };
  },
  returnToHub: async (): Promise<void> => {
    await ipcRenderer.invoke("litecode:return-to-hub");
  },
  getUiTheme: (): "default" | "light" => {
    const theme = ipcRenderer.sendSync("litecode:get-ui-theme") as string | undefined;
    return theme === "light" ? "light" : "default";
  },
  setUiTheme: async (theme: "default" | "light"): Promise<void> => {
    await ipcRenderer.invoke("litecode:set-ui-theme", theme);
  },
  windowMinimize: async (): Promise<void> => {
    await ipcRenderer.invoke("litecode:window-minimize");
  },
  windowMaximizeToggle: async (): Promise<boolean> => {
    return (await ipcRenderer.invoke("litecode:window-maximize-toggle")) as boolean;
  },
  windowIsMaximized: async (): Promise<boolean> => {
    return (await ipcRenderer.invoke("litecode:window-is-maximized")) as boolean;
  },
  windowClose: async (): Promise<void> => {
    await ipcRenderer.invoke("litecode:window-close");
  },
  popoutSetAlwaysOnTop: async (dockId: string, onTop: boolean): Promise<boolean> => {
    return (await ipcRenderer.invoke("litecode:popout-set-always-on-top", dockId, onTop)) === true;
  },
  browserCreate: async (input: {
    id: string;
    backgroundColor: string;
  }): Promise<BrowserState> => {
    return (await ipcRenderer.invoke("litecode:browser-create", input)) as BrowserState;
  },
  browserNavigate: async (input: { id: string; url: string }): Promise<BrowserState> => {
    return (await ipcRenderer.invoke("litecode:browser-navigate", input)) as BrowserState;
  },
  browserGoBack: async (id: string): Promise<BrowserState> => {
    return (await ipcRenderer.invoke("litecode:browser-go-back", id)) as BrowserState;
  },
  browserGoForward: async (id: string): Promise<BrowserState> => {
    return (await ipcRenderer.invoke("litecode:browser-go-forward", id)) as BrowserState;
  },
  browserReload: async (id: string): Promise<BrowserState> => {
    return (await ipcRenderer.invoke("litecode:browser-reload", id)) as BrowserState;
  },
  browserStop: async (id: string): Promise<BrowserState> => {
    return (await ipcRenderer.invoke("litecode:browser-stop", id)) as BrowserState;
  },
  browserSetBounds: (input: {
    id: string;
    bounds: BrowserBounds;
    place?: string;
  }): void => {
    ipcRenderer.send("litecode:browser-set-bounds", input);
  },
  browserSetHost: (input: { id: string; popoutId: string | null }): boolean => {
    return ipcRenderer.sendSync("litecode:browser-set-host", input) === true;
  },
  browserSetVisible: (input: { id: string; visible: boolean }): void => {
    ipcRenderer.send("litecode:browser-set-visible", input);
  },
  browserSetActive: (input: { id: string; active: boolean }): void => {
    ipcRenderer.send("litecode:browser-set-active", input);
  },
  browserDestroy: (id: string): void => {
    ipcRenderer.send("litecode:browser-destroy", id);
  },
  browserSetObscured: (obscured: boolean): void => {
    ipcRenderer.sendSync("litecode:browser-set-obscured", obscured);
  },
  onBrowserState: (handler: (state: BrowserState) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: BrowserState) => {
      handler(payload);
    };
    ipcRenderer.on(BROWSER_STATE_CHANNEL, listener);
    return () => ipcRenderer.removeListener(BROWSER_STATE_CHANNEL, listener);
  },
  onBrowserOpenRequest: (handler: (event: BrowserOpenRequest) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: BrowserOpenRequest) => {
      handler(payload);
    };
    ipcRenderer.on(BROWSER_OPEN_CHANNEL, listener);
    return () => ipcRenderer.removeListener(BROWSER_OPEN_CHANNEL, listener);
  },
  onBrowserCloseRequest: (handler: (event: BrowserCloseRequest) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: BrowserCloseRequest) => {
      handler(payload);
    };
    ipcRenderer.on(BROWSER_CLOSE_CHANNEL, listener);
    return () => ipcRenderer.removeListener(BROWSER_CLOSE_CHANNEL, listener);
  },
  onBrowserAgentActivity: (
    handler: (event: BrowserAgentActivityEvent) => void,
  ): (() => void) => {
    const listener = (
      _event: Electron.IpcRendererEvent,
      payload: BrowserAgentActivityEvent,
    ) => {
      handler(payload);
    };
    ipcRenderer.on(BROWSER_AGENT_ACTIVITY_CHANNEL, listener);
    return () => ipcRenderer.removeListener(BROWSER_AGENT_ACTIVITY_CHANNEL, listener);
  },
} satisfies LitecodeDesktopBridge;

contextBridge.exposeInMainWorld("litecode", bridge);
