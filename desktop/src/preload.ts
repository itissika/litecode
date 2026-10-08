import { contextBridge, ipcRenderer, webUtils } from "electron";

/**
 * Sandboxed preload may only `require` Electron/Node builtins — not local files.
 * Keep this string identical to `REMOTE_PROGRESS_CHANNEL` in `./remote-progress`.
 */
const REMOTE_PROGRESS_CHANNEL = "litecode:remote-progress";
/** Keep identical to `BROWSER_STATE_CHANNEL` in `./browser-host`. */
const BROWSER_STATE_CHANNEL = "litecode:browser-state";

type RemoteProgressEvent = {
  stage: string;
  ratio: number;
  message: string;
};

type RecentWorkspace = {
  path: string;
  pinned: boolean;
  lastOpenedAt: number;
};

type BrowserBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type BrowserState = {
  id: string;
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
};

type RemoteHistoryItem = {
  id: string;
  label: string;
  host: string;
  user?: string;
  port?: number;
  lastWorkspace?: string;
  lastConnectedAt?: number;
  pinned?: boolean;
};

contextBridge.exposeInMainWorld("litecode", {
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
  startRemoteSession: async (input: {
    userAtHost: string;
    password?: string;
    authMode?: "password" | "private_key" | "agent";
    identityFile?: string;
    label?: string;
  }): Promise<{ sessionId: string; home: string; label: string }> => {
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
  focusWorkspace: async (workspacePath: string): Promise<boolean> => {
    return (await ipcRenderer.invoke("litecode:focus-workspace", workspacePath)) as boolean;
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
});
