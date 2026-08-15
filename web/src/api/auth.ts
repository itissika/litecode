/** Host-injected or dev-only auth token for local serve. Never a user settings field. */

export interface LitecodeDesktopBridge {
  getAuthToken?: () => string | undefined;
  /** Electron session: local sidecar vs remote attach. */
  getSessionMode?: () => "local" | "remote";
  pickFolder?: () => Promise<string | null>;
  listRecents?: () => Promise<Array<{ path: string; pinned: boolean; lastOpenedAt: number }>>;
  setRecentPinned?: (
    workspacePath: string,
    pinned: boolean,
  ) => Promise<Array<{ path: string; pinned: boolean; lastOpenedAt: number }>>;
  removeRecent?: (workspacePath: string) => Promise<Array<{ path: string; pinned: boolean; lastOpenedAt: number }>>;
  listSshTargets?: () => Promise<Array<{
    id: string;
    label: string;
    host: string;
    user?: string;
    port?: number;
    identityFile?: string;
    lastWorkspace?: string;
    lastConnectedAt?: number;
  }>>;
  saveSshTarget?: (target: {
    id?: string;
    label: string;
    host: string;
    user?: string;
    port?: number;
    identityFile?: string;
  }) => Promise<{ id: string; label: string; host: string }>;
  removeSshTarget?: (id: string) => Promise<void>;
  listSshDirectory?: (
    targetId: string,
    remotePath?: string,
  ) => Promise<{ path: string; entries: Array<{ name: string }> }>;
  connectSshWorkspace?: (
    targetId: string,
    workspace: string,
  ) => Promise<{ ok: boolean; mode: "remote" }>;
  /** Ask another live instance that owns this workspace to focus; returns true if focused. */
  focusWorkspace?: (workspacePath: string) => Promise<boolean>;
  /** Advertise the workspace this window currently owns (for multi-instance focus). */
  notifyWorkspace?: (workspacePath: string | null) => Promise<void>;
  /**
   * Open a folder: focus existing instance, or relaunch this window's sidecar
   * with `--workspace` (real process restart). Not an in-process HTTP switch.
   * Rejected in remote mode.
   */
  openWorkspace?: (
    workspacePath: string,
  ) => Promise<{ ok: boolean; focused?: boolean; project: string }>;
  /** Attach to a remote serve (no local sidecar). */
  connectRemote?: (
    baseUrl: string,
    token: string,
  ) => Promise<{ ok: boolean; mode: "remote" }>;
  /** Leave remote and spawn a local sidecar for the given workspace. */
  returnToLocal?: (
    workspacePath: string,
  ) => Promise<{ ok: boolean; focused?: boolean; project: string }>;
  /** Stop the active local session, if any, and return to the startup hub. */
  returnToHub?: () => Promise<void>;
  /** Shared UI theme (hub + workbench); persisted in Electron userData. */
  getUiTheme?: () => "default" | "light";
  setUiTheme?: (theme: "default" | "light") => Promise<void>;
  /** Home remote wizard (managed SSH). */
  listRemoteHistory?: () => Promise<
    Array<{
      id: string;
      label: string;
      host: string;
      user?: string;
      port?: number;
      lastWorkspace?: string;
      pinned?: boolean;
    }>
  >;
  startRemoteSession?: (input: {
    userAtHost: string;
    password?: string;
    authMode?: "password" | "private_key" | "agent";
    identityFile?: string;
    label?: string;
  }) => Promise<{ sessionId: string; home: string; label: string }>;
  listPendingRemoteDirs?: (
    sessionId: string,
    remotePath?: string,
  ) => Promise<{ path: string; home: string; entries: Array<{ name: string }> }>;
  completeRemoteSession?: (
    sessionId: string,
    workspace: string,
  ) => Promise<{ token: string; baseUrl: string; workspace: string; label: string }>;
  enterRemoteWorkbench?: (sessionId: string) => Promise<{ ok: boolean; mode: "remote" }>;
  cancelRemoteSession?: (sessionId: string) => Promise<void>;
  reconnectRemote?: (id: string) => Promise<{ ok: boolean; mode: "remote" }>;
  onRemoteProgress?: (
    handler: (event: {
      stage: string;
      ratio: number;
      message: string;
    }) => void,
  ) => () => void;
  /** Electron frameless chrome (no-ops in browser). */
  windowMinimize?: () => Promise<void>;
  windowMaximizeToggle?: () => Promise<boolean>;
  windowIsMaximized?: () => Promise<boolean>;
  windowClose?: () => Promise<void>;
}

declare global {
  interface Window {
    litecode?: LitecodeDesktopBridge;
  }
}

/** Read `?token=` from the page URL (browser handshake links from serve_win.ps1). */
export function tokenFromPageUrl(
  search: string = typeof window !== "undefined" ? window.location.search : "",
): string | undefined {
  try {
    const token = new URLSearchParams(search).get("token");
    if (typeof token === "string" && token.length > 0) {
      return token;
    }
  } catch {
    // ignore malformed search
  }
  return undefined;
}

/**
 * Resolve the process auth token without user input.
 * Priority: Electron preload bridge → page `?token=` → Vite dev env.
 */
export function getAuthToken(): string | undefined {
  const fromHost = window.litecode?.getAuthToken?.();
  if (typeof fromHost === "string" && fromHost.length > 0) {
    return fromHost;
  }
  const fromUrl = tokenFromPageUrl();
  if (fromUrl) {
    return fromUrl;
  }
  const fromEnv = import.meta.env.VITE_AUTH_TOKEN;
  if (typeof fromEnv === "string" && fromEnv.length > 0) {
    return fromEnv;
  }
  return undefined;
}

/** Merge Authorization: Bearer when a token is available. */
export function withAuthHeaders(init?: HeadersInit): Headers {
  const headers = new Headers(init);
  const token = getAuthToken();
  if (token && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  return headers;
}

export async function apiFetch(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const headers = withAuthHeaders(init?.headers);
  return fetch(input, { ...init, headers });
}
