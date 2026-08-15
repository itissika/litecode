import { contextBridge, ipcRenderer } from "electron";

/**
 * Sandboxed preload may only `require` Electron/Node builtins — not local files.
 * Keep this string identical to `REMOTE_PROGRESS_CHANNEL` in `./remote-progress`.
 */
const REMOTE_PROGRESS_CHANNEL = "litecode:remote-progress";

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
  getSessionMode: (): "local" | "remote" => {
    const mode = ipcRenderer.sendSync("litecode:get-session-mode") as string | undefined;
    return mode === "remote" ? "remote" : "local";
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
});
