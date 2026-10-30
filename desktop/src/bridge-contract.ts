/**
 * Desktop preload bridge contract (WP-161). Single source of truth for the
 * `window.litecode` API exposed by `preload.ts`.
 *
 * - Types: `export type`. web (WP-162) consumes these with `import type` only.
 * - Channel names: `export const`. Desktop-internal; web must not import them.
 *
 * NOTE: the sandboxed preload cannot `require` local files, so `preload.ts`
 * only uses `import type` from here and pins its channel literals with
 * `typeof <CONST>` — a drift becomes a compile error.
 */

/* ------------------------------------------------------------------ */
/* Push channels (main -> renderer)                                    */
/* ------------------------------------------------------------------ */

export const REMOTE_PROGRESS_CHANNEL = "litecode:remote-progress";
export const COMPONENT_PROGRESS_CHANNEL = "litecode:component-progress";
export const BROWSER_STATE_CHANNEL = "litecode:browser-state";
export const APP_VIEW_STATE_CHANNEL = "litecode:app-view-state";
/** renderer -> main: hide/show one native view behind a workbench surface. */
export const NATIVE_VIEW_SET_OCCLUDED_CHANNEL = "litecode:native-view-set-occluded";
/** renderer -> main: latest still frame for a native view. */
export const NATIVE_VIEW_GET_SNAPSHOT_CHANNEL = "litecode:native-view-get-snapshot";
/** main -> renderer: open a visible browser panel. */
export const BROWSER_OPEN_CHANNEL = "litecode:browser-open";
/** main -> renderer: close one browser panel. */
export const BROWSER_CLOSE_CHANNEL = "litecode:browser-close";
/** main -> renderer: the agent is operating this page; disable human controls. */
export const BROWSER_AGENT_ACTIVITY_CHANNEL = "litecode:browser-agent-activity";
/** Turn-finished toast clicked. main -> renderer, one-way: `{ sessionId }`. */
export const TURN_NOTIFICATION_CLICK_CHANNEL = "litecode:turn-notification-click";

/**
 * Remote session lost. main -> renderer, one-way.
 *
 * Triggers:
 * - SSH tunnel disconnected unexpectedly ("SSH tunnel disconnected.");
 * - tunnel auto-reconnect failed (the error message);
 * - local sidecar crashed while in remote mode ("Local sidecar crashed.").
 *
 * Payload `{ reason: string }`: diagnostic only; the UI shows no text for it.
 *
 * There is deliberately NO "reconnected" event. A successful reconnect reloads
 * the whole window, and that reload is the only recovery path. The renderer's
 * disconnected state is set only by this event and cleared only by reload.
 */
export const REMOTE_DISCONNECTED_CHANNEL = "litecode:remote-disconnected";

export type RemoteDisconnectedEvent = { reason: string };

/* ------------------------------------------------------------------ */
/* Event / data shapes                                                 */
/* ------------------------------------------------------------------ */

type RemoteProgressStage =
  | "authenticating"
  | "download"
  | "upload"
  | "verify"
  | "extract"
  | "ready"
  | "starting"
  | "attaching";

export type RemoteProgressEvent = {
  stage: RemoteProgressStage;
  /** 0..1 */
  ratio: number;
  message: string;
};

export type ComponentId = "embed-model" | "linux-server";

export type ComponentPhase = "resolving" | "downloading" | "verifying" | "installing" | "done" | "error";

/** Download lifecycle status. abortAll (quit) yields "paused". */
export type ComponentDownloadStatus = "idle" | "downloading" | "paused" | "error" | "done";

export type ComponentProgressEvent = {
  id: ComponentId;
  phase: ComponentPhase;
  status?: ComponentDownloadStatus;
  /** Bytes received (downloading/verifying). */
  received?: number;
  /** Expected bytes. */
  total?: number;
  /** 0..100, always present (0 when total is unknown). */
  percent: number;
  /** Sliding-window throughput while downloading. */
  bytesPerSecond?: number;
  message: string;
  error?: string;
};

export type ComponentStatus = {
  id: ComponentId;
  installed: boolean;
  path: string | null;
  downloading: boolean;
  status: ComponentDownloadStatus;
  error: string | null;
};

export type EnsureComponentResult = { id: ComponentId; path: string; downloaded: boolean };

export type RecentWorkspace = {
  path: string;
  pinned: boolean;
  lastOpenedAt: number;
};

export type RemoteHistoryItem = {
  id: string;
  label: string;
  host: string;
  user?: string;
  port?: number;
  lastWorkspace?: string;
  lastConnectedAt?: number;
  pinned?: boolean;
};

export type BrowserBounds = { x: number; y: number; width: number; height: number };

export type BrowserState = {
  id: string;
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
};

export type AppViewState = BrowserState;
export type NativeViewKind = "browser" | "app";
export type NativeViewRef = { kind: NativeViewKind; id: string };
export type NativeViewOccludedInput = NativeViewRef & { occluded: boolean };
export type AppViewCreateInput = {
  id: string;
  appId?: string;
  viewKey: string;
  url: string;
  backgroundColor: string;
  permissions: string[];
};

export type BrowserOpenRequest = { id: string; url: string };
export type BrowserCloseRequest = { id: string };
export type BrowserAgentActivityEvent = { id: string; busy: boolean };

export type TurnNotificationPayload = {
  sessionId: string;
  turnId: string;
  title: string;
  body: string;
};

export type TurnNotificationClickEvent = { sessionId: string };

type SessionMode = "local" | "remote";
type UiTheme = "default" | "light";

export type StartRemoteSessionInput = {
  userAtHost: string;
  password?: string;
  authMode?: "password" | "private_key" | "agent";
  identityFile?: string;
  label?: string;
};

type Unsubscribe = () => void;

/* ------------------------------------------------------------------ */
/* Bridge                                                              */
/* ------------------------------------------------------------------ */

/** `window.litecode` as exposed by preload.ts. */
export type LitecodeDesktopBridge = {
  getAuthToken(): string | undefined;
  getPathForFile(file: File): string;
  getSessionMode(): SessionMode;
  loadLayout(): string | null;
  saveLayout(payload: string): void;
  pickFolder(): Promise<string | null>;
  listRecents(): Promise<RecentWorkspace[]>;
  setRecentPinned(workspacePath: string, pinned: boolean): Promise<RecentWorkspace[]>;
  removeRecent(workspacePath: string): Promise<RecentWorkspace[]>;
  listRemoteHistory(): Promise<RemoteHistoryItem[]>;
  setRemoteHistoryPinned(id: string, pinned: boolean): Promise<void>;
  removeSshTarget(id: string): Promise<void>;
  startRemoteSession(
    input: StartRemoteSessionInput,
  ): Promise<{ sessionId: string; home: string; label: string }>;
  listPendingRemoteDirs(
    sessionId: string,
    remotePath?: string,
  ): Promise<{ path: string; home: string; entries: Array<{ name: string }> }>;
  completeRemoteSession(
    sessionId: string,
    workspace: string,
  ): Promise<{ token: string; baseUrl: string; workspace: string; label: string }>;
  enterRemoteWorkbench(sessionId: string): Promise<{ ok: boolean; mode: "remote" }>;
  cancelRemoteSession(sessionId: string): Promise<void>;
  reconnectRemote(id: string): Promise<{ ok: boolean; mode: "remote" }>;
  onRemoteProgress(handler: (event: RemoteProgressEvent) => void): Unsubscribe;
  /** See REMOTE_DISCONNECTED_CHANNEL. */
  onRemoteDisconnected(handler: (event: RemoteDisconnectedEvent) => void): Unsubscribe;
  getComponentStatus(id: string): Promise<ComponentStatus>;
  ensureComponent(id: string): Promise<EnsureComponentResult>;
  onComponentProgress(handler: (event: ComponentProgressEvent) => void): Unsubscribe;
  notifyWorkspace(workspacePath: string | null): Promise<void>;
  /** Desktop toast for a finished turn. The renderer does the visibility gate. */
  notifyTurnEnd(payload: TurnNotificationPayload): void;
  /** See TURN_NOTIFICATION_CLICK_CHANNEL. */
  onTurnNotificationClick(handler: (event: TurnNotificationClickEvent) => void): Unsubscribe;
  openWorkspace(workspacePath: string): Promise<{ ok: boolean; focused?: boolean; project: string }>;
  returnToHub(): Promise<void>;
  getUiTheme(): UiTheme;
  setUiTheme(theme: UiTheme): Promise<void>;
  windowMinimize(): Promise<void>;
  windowMaximizeToggle(): Promise<boolean>;
  windowIsMaximized(): Promise<boolean>;
  windowClose(): Promise<void>;
  popoutSetAlwaysOnTop(dockId: string, onTop: boolean): Promise<boolean>;
  /** Window controls for one popout window, addressed by its dock id. */
  popoutWindowMinimize(dockId: string): Promise<void>;
  /** Returns whether the window is maximized after the toggle. */
  popoutWindowMaximizeToggle(dockId: string): Promise<boolean>;
  popoutWindowIsMaximized(dockId: string): Promise<boolean>;
  browserCreate(input: { id: string; backgroundColor: string }): Promise<BrowserState>;
  browserNavigate(input: { id: string; url: string }): Promise<BrowserState>;
  browserGoBack(id: string): Promise<BrowserState>;
  browserGoForward(id: string): Promise<BrowserState>;
  browserReload(id: string): Promise<BrowserState>;
  browserStop(id: string): Promise<BrowserState>;
  browserSetBounds(input: { id: string; bounds: BrowserBounds; place?: string }): void;
  browserSetHost(input: { id: string; popoutId: string | null }): boolean;
  browserSetVisible(input: { id: string; visible: boolean }): void;
  browserSetActive(input: { id: string; active: boolean }): void;
  browserDestroy(id: string): void;
  browserSetObscured(obscured: boolean): void;
  nativeViewSetOccluded(input: NativeViewOccludedInput): void;
  nativeViewGetSnapshot(input: NativeViewRef): Promise<string>;
  appViewCreate(input: AppViewCreateInput): Promise<AppViewState>;
  appViewGoBack(id: string): Promise<AppViewState>;
  appViewGoForward(id: string): Promise<AppViewState>;
  appViewReload(id: string): Promise<AppViewState>;
  appViewStop(id: string): Promise<AppViewState>;
  appViewSetBounds(input: { id: string; bounds: BrowserBounds }): void;
  appViewSetVisible(input: { id: string; visible: boolean }): void;
  appViewSetActive(input: { id: string; active: boolean }): void;
  appViewDestroy(id: string): void;
  onAppViewState(handler: (state: AppViewState) => void): Unsubscribe;
  onBrowserState(handler: (state: BrowserState) => void): Unsubscribe;
  onBrowserOpenRequest(handler: (event: BrowserOpenRequest) => void): Unsubscribe;
  onBrowserCloseRequest(handler: (event: BrowserCloseRequest) => void): Unsubscribe;
  onBrowserAgentActivity(
    handler: (event: BrowserAgentActivityEvent) => void,
  ): Unsubscribe;
};
