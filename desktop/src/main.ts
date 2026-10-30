import crypto from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { pathToFileURL } from "node:url";

import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  type MessageBoxOptions,
  Notification,
  screen,
  shell,
} from "electron";

import { InstanceRegistry, normalizeWorkspace } from "./instance-registry";
import { handshakeUrl } from "./handshake-url";
import {
  assertDevUiConfigured,
  devUiDocumentUrl,
  devUpstreamFilePath,
  sameWorkbenchDocument,
  syncDevUpstream,
} from "./dev-ui";
import { readWorkspaceLayout, writeWorkspaceLayout } from "./layout-file";
import { boundsOnDisplay, popoutBoundsFromFeatures, type PopoutBounds } from "./popout-bounds";
import { writeHubPage } from "./hub";
import { initDesktopI18n, resolveLocale, t } from "./i18n";
import {
  assertIpcSurface,
  exactHttpOrigin,
  isAllowedNavigation,
  isPopoutDockId,
  resolveWindowOpen,
  workbenchPopoutDockId,
  type AllowedSurface,
  type IpcTrustContext,
} from "./ipc-trust";
import { formatUserAtHost, parseUserAtHost } from "./parse-user-host";
import {
  clearAllPendingRemotes,
  createPendingId,
  deletePendingRemote,
  getPendingRemote,
  setPendingRemote,
  takePendingRemote,
  type PendingRemoteSession,
} from "./pending-remote";
import { RecentWorkspaces, type RecentWorkspace } from "./recents";
import { deleteSecret, getSecret, setSecret } from "./secure-store";
import { RemoteTargets, type SavedRemoteTarget } from "./remote-targets";
import {
  ensureComponentForRemote as ensureComponentForRemoteCore,
  type RemoteProgressEvent,
} from "./remote-progress";
import { LifecycleMutex, SingleFlight } from "./lifecycle-mutex";
import { clearManagedRemoteIfSession } from "./remote-state";
import {
  DEFAULT_SIDECAR_RECOVERY,
  DEFAULT_TUNNEL_RECOVERY,
  RecoveryCounter,
  recoveryDelayMs,
} from "./process-recovery";
import { isBootAborted, runBootSequence } from "./boot-sequence";
import { startSidecar, stopSidecar, watchSidecarExit, type SidecarHandle } from "./sidecar";
import {
  BUNDLED_MODEL_REL,
  LINUX_TAR_NAME,
  devLinuxBundleTar,
  devRepoRoot as devRepoRootFor,
  resolveBundledModelDir,
  resolveLinuxBundle,
} from "./bundle-paths";
import { sidecarRoot } from "./paths";
import {
  BROWSER_AGENT_ACTIVITY_CHANNEL,
  BROWSER_CLOSE_CHANNEL,
  COMPONENT_PROGRESS_CHANNEL,
  NATIVE_VIEW_GET_SNAPSHOT_CHANNEL,
  NATIVE_VIEW_SET_OCCLUDED_CHANNEL,
  REMOTE_DISCONNECTED_CHANNEL,
  REMOTE_PROGRESS_CHANNEL,
  TURN_NOTIFICATION_CLICK_CHANNEL,
} from "./bridge-contract";
import {
  ComponentManager,
  isComponentId,
  type ComponentId,
  type ComponentProgressEvent,
  type EnsureResult,
} from "./components";
import {
  materializePasswordAskPass,
  materializePrivateKeyFile,
  readPrivateKeyFile,
  removeMaterializedKey,
  withMaterializedKeyDirectory,
} from "./ssh-credential-material";
import {
  scanSshHostKey,
  SshSession,
  type RemoteServeHandle,
  type SshTunnelHandle,
  type TunnelExitInfo,
} from "./ssh-session";
import { SshTargets, type SavedSshTarget } from "./ssh-targets";
import { readUiTheme, writeUiTheme, type UiThemeName } from "./ui-theme";
import { httpOrigin, isAllowedExternalUrl, isAllowedLoadUrl, isLoopbackHttpUrl, sameAppHttpOrigin } from "./url-policy";
import { senderOwnedWindow } from "./window-guard";
import { BrowserHost } from "./browser-host";
import { configureAppSession, createAppViewHost, createBrowserHost } from "./browser-view";
import { createBrowserAutomation } from "./browser-automation";
import {
  BrowserBridgeError,
  startBrowserBridge,
  type BrowserBridge,
} from "./browser-bridge";

type SessionMode = "local" | "remote";

let mainWindow: BrowserWindow | null = null;
/** Must match `build.appId`: the NSIS Start Menu shortcut carries this AUMID. */
const DESKTOP_APP_ID = "app.litecode.desktop";
/** Dev electron.exe must not claim the installed app's AUMID or its icon is poisoned. */
const DESKTOP_DEV_APP_ID = `${DESKTOP_APP_ID}.dev`;
/** Recent toasts stay referenced so a click from Action Center still lands. */
const liveTurnToasts: Notification[] = [];
const MAX_LIVE_TOASTS = 5;
let browserHost: BrowserHost | null = null;
let appViewHost: BrowserHost | null = null;
const appViewOrigins = new Map<string, string>();
const appViewPartitions = new Map<string, string>();
let browserBridge: BrowserBridge | null = null;
/**
 * OS handles for popout windows, keyed by the dock id in `/popout.html?dock=`.
 * The renderer keeps the matching DOM windows. A handle may exist before
 * Dockview's group, and the group may be seen first. Always-on-top and the
 * embedded browser fail closed when the handle is missing; they do not wait.
 * Minimize, maximize, and close follow the IPC sender, not this map.
 */
const popoutWindows = new Map<string, BrowserWindow>();
let browserObscured = false;
let sidecar: SidecarHandle | null = null;
let authToken = "";
let sessionMode: SessionMode = "local";
let registry: InstanceRegistry | null = null;
let recents: RecentWorkspaces | null = null;
let remoteTargets: RemoteTargets | null = null;
let sshTargets: SshTargets | null = null;

type ManagedRemote = {
  session: SshSession;
  serve: RemoteServeHandle;
  tunnel: SshTunnelHandle;
  keyFile?: string;
  keyDirectory?: string;
  /** SSH target id for limited auto-reconnect (D1). */
  targetId?: string;
  workspace?: string;
};

let managedRemote: ManagedRemote | null = null;
let quitting = false;
let currentWorkspace: string | null = null;
let trustContext: IpcTrustContext | null = null;

/** Serialise sidecar stop/start so concurrent relaunch cannot orphan (D5). */
const sidecarLifecycle = new LifecycleMutex();
/** Single-flight Open Remote start (D4). */
const openRemoteStartFlight = new SingleFlight<{ sessionId: string; home: string; label: string }>();
/** Single-flight Open Remote complete (D4). */
const openRemoteCompleteFlight = new SingleFlight<{
  token: string;
  baseUrl: string;
  workspace: string;
  label: string;
}>();
const tunnelRecovery = new RecoveryCounter();
const sidecarRecovery = new RecoveryCounter();
let tunnelReconnectTimer: NodeJS.Timeout | null = null;
let sidecarRestartTimer: NodeJS.Timeout | null = null;

function createToken(): string {
  return crypto.randomBytes(32).toString("base64url");
}

type BootContent = { kind: "hub" } | { kind: "skeleton" };

/**
 * WP-169 skeleton: theme background only, whole page draggable (frameless
 * window), no text, no script. A data: URL is never a trusted IPC surface
 * (ipc-trust only trusts the hub file URL and the workbench origin).
 */
const SKELETON_URL =
  "data:text/html;charset=utf-8," +
  encodeURIComponent(
    '<!doctype html><html><head><meta charset="utf-8"></head>' +
      '<body style="margin:0;width:100vw;height:100vh;background:#0a0a0a;-webkit-app-region:drag"></body></html>',
  );

function hubPagePath(): string {
  return path.join(app.getPath("userData"), "hub-index.html");
}

function disposeAppViewHost(): void {
  appViewHost?.dispose();
  appViewHost?.destroyAll();
  appViewHost = null;
  appViewOrigins.clear();
  appViewPartitions.clear();
}

function disposeBrowserHost(): void {
  browserHost?.dispose();
  browserHost?.destroyAll();
  browserHost = null;
  disposeAppViewHost();
}

function ensureBrowserHost(): BrowserHost | null {
  const win = mainWindow;
  if (!win || win.isDestroyed()) return null;
  if (!browserHost) {
    browserHost = createBrowserHost(win, (id) => popoutWindows.get(id) ?? null);
    browserHost.setObscured(browserObscured);
  }
  return browserHost;
}

function appViewPartition(viewKey: string): string {
  const clean = viewKey.replace(/[^a-zA-Z0-9_-]/g, "-").slice(0, 120);
  if (!clean) throw new Error("Invalid app view key");
  return `persist:litecode-app-${clean}`;
}

function ensureAppViewHost(): BrowserHost | null {
  const win = mainWindow;
  if (!win || win.isDestroyed()) return null;
  if (!appViewHost) {
    appViewHost = createAppViewHost(win, {
      partitionFor: (id) =>
        appViewPartitions.get(id) ?? "persist:litecode-app-unknown",
      isAllowedUrl: (url, id) => {
        const origin = appViewOrigins.get(id);
        return origin !== undefined && sameAppHttpOrigin(url, origin);
      },
      onExternal: (url) => {
        if (isAllowedExternalUrl(url)) void shell.openExternal(url);
      },
    });
    appViewHost.setObscured(browserObscured);
  }
  return appViewHost;
}

function sendToWorkbench(channel: string, payload: unknown): void {
  if (
    channel === BROWSER_AGENT_ACTIVITY_CHANNEL ||
    channel === BROWSER_CLOSE_CHANNEL
  ) {
    // A browser panel can live in a popout; every workbench window must see
    // takeover state and close requests for its own dockview.
    for (const win of [mainWindow, ...popoutWindows.values()]) {
      if (!win || win.isDestroyed() || win.webContents.isDestroyed()) continue;
      win.webContents.send(channel, payload);
    }
    return;
  }
  const win = mainWindow;
  if (!win || win.isDestroyed() || win.webContents.isDestroyed()) {
    throw new BrowserBridgeError(
      "unavailable",
      "The workbench window is not ready",
      503,
    );
  }
  win.webContents.send(channel, payload);
}

/**
 * Start the loopback bridge before booting the sidecar. The sidecar inherits
 * these variables from process.env, which is the only place they belong.
 */
async function startBrowserAutomationBridge(): Promise<void> {
  if (browserBridge) return;
  try {
    browserBridge = await startBrowserBridge(
      createBrowserAutomation({
        getHost: () => ensureBrowserHost(),
        send: sendToWorkbench,
        busyLabel: () => t("browser.agentBusy"),
      }),
    );
    process.env.LITECODE_BROWSER_BRIDGE_URL = browserBridge.url;
    process.env.LITECODE_BROWSER_BRIDGE_TOKEN = browserBridge.token;
  } catch (error) {
    delete process.env.LITECODE_BROWSER_BRIDGE_URL;
    delete process.env.LITECODE_BROWSER_BRIDGE_TOKEN;
    console.error("Could not start the local browser bridge", error);
  }
}

function displayWorkAreas(): PopoutBounds[] {
  return screen.getAllDisplays().map((display) => display.workArea);
}

/** Screen rectangle from `window.open`, pulled onto a display when the opener is minimized. */
function openedPopoutBounds(features: string | undefined): PopoutBounds | null {
  const raw = popoutBoundsFromFeatures(features);
  if (!raw) return null;
  return boundsOnDisplay(raw, displayWorkAreas(), screen.getCursorScreenPoint());
}

/**
 * Put a popout on screen and show it.
 * On Windows a window opened while its owner is minimized is created minimized.
 * The taskbar thumbnail still paints the restored page, so the preview looks
 * fine while the desktop shows nothing. Detach it from that owner, or restore()
 * is undone immediately.
 */
function presentPopout(child: BrowserWindow, requested: PopoutBounds | null): void {
  const place = () => {
    if (child.isDestroyed()) return;
    if (child.getParentWindow()) child.setParentWindow(null);
    if (child.isMinimized()) child.restore();
    const current = child.getBounds();
    const next = boundsOnDisplay(
      requested ?? current,
      displayWorkAreas(),
      screen.getCursorScreenPoint(),
    );
    const moved =
      next.x !== current.x ||
      next.y !== current.y ||
      next.width !== current.width ||
      next.height !== current.height;
    if (moved) child.setBounds(next);
    const opener = mainWindow;
    const openerHidden =
      !opener || opener.isDestroyed() || opener.isMinimized() || !opener.isVisible();
    if (openerHidden || !child.isVisible()) {
      child.show();
      child.moveTop();
      child.focus();
    }
  };
  place();
  child.once("ready-to-show", place);
  const opener = mainWindow;
  if (opener && !opener.isDestroyed() && opener.isMinimized()) {
    setTimeout(place, 0);
    setTimeout(place, 80);
  }
}

function popoutWindowOptions(): Electron.BrowserWindowConstructorOptions {
  const iconPath = path.join(__dirname, "..", "build", "icon.ico");
  return {
    // No OS title bar. The popout is dragged from the tab-bar void; thickFrame
    // (Windows default) keeps the resize edges.
    frame: false,
    show: true,
    autoHideMenuBar: true,
    backgroundColor: "#0a0a0a",
    icon: fs.existsSync(iconPath) ? iconPath : undefined,
    webPreferences: {
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      preload: undefined,
      // Same-origin window.open shares the main renderer. Keep this window
      // painting if Chromium marks it hidden while the opener is off-screen.
      backgroundThrottling: false,
    },
  };
}

function lockPopoutWindow(child: BrowserWindow, openedUrl: string): void {
  child.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  const guard = (event: Electron.Event, url: string) => {
    if (url !== openedUrl) event.preventDefault();
  };
  child.webContents.on("will-navigate", guard);
  child.webContents.on("will-redirect", guard);
}

function rememberPopout(dock: string, child: BrowserWindow): void {
  popoutWindows.set(dock, child);
  child.once("close", () => {
    if (popoutWindows.get(dock) === child) popoutWindows.delete(dock);
    browserHost?.parkPopout(dock);
  });
}

function asRecord(payload: unknown): Record<string, unknown> {
  if (!payload || typeof payload !== "object") return {};
  return payload as Record<string, unknown>;
}

async function loadHub(win: BrowserWindow): Promise<void> {
  disposeBrowserHost();
  const filePath = writeHubPage(hubPagePath());
  trustContext = {
    activeSurface: "hub",
    hubFileUrl: pathToFileURL(filePath).toString(),
    workbenchOrigin: trustContext?.workbenchOrigin ?? null,
  };
  await win.loadFile(filePath);
}

/** G5: loadURL may only reach http(s); refuse anything else. */
async function safeLoadURL(win: BrowserWindow, url: string): Promise<void> {
  if (!isAllowedLoadUrl(url)) {
    throw new Error(`Refusing to load a non-http(s) URL: ${url}`);
  }
  const workbenchOrigin = exactHttpOrigin(url);
  if (!workbenchOrigin) {
    throw new Error(`Refusing to load an untrusted workbench URL: ${url}`);
  }
  disposeBrowserHost();
  trustContext = {
    activeSurface: "workbench",
    hubFileUrl: pathToFileURL(hubPagePath()).toString(),
    workbenchOrigin,
  };
  await win.loadURL(url);
}

async function createWindow(content: BootContent): Promise<BrowserWindow> {
  const iconPath = path.join(__dirname, "..", "build", "icon.ico");
  const win = new BrowserWindow({
    width: 1280,
    height: 800,
    show: false,
    frame: false,
    icon: fs.existsSync(iconPath) ? iconPath : undefined,
    backgroundColor: "#0a0a0a",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      // Popouts are same-process children of this window. The default throttle
      // freezes that process — and every popout, including resize — when this
      // window is minimized, occluded, or hidden.
      backgroundThrottling: false,
    },
  });
  // Preload/page scripts can invoke synchronous IPC while loadFile/loadURL is
  // still resolving. Register the window before navigation so those first
  // trusted calls are not rejected by requireTrustedIpc().
  mainWindow = win;

  win.once("ready-to-show", () => win.show());
  win.webContents.on("preload-error", (_event, preloadPath, error) => {
    console.error(`[litecode] preload failed (${preloadPath}):`, error);
  });
  win.webContents.setWindowOpenHandler(({ url, features }) => {
    const decision = resolveWindowOpen(url, trustContext);
    if (decision === "popout") {
      // overrideBrowserWindowOptions replaces the window.open feature string,
      // which is where dockview put the saved screen rectangle.
      const bounds = openedPopoutBounds(features);
      return {
        action: "allow",
        overrideBrowserWindowOptions: { ...popoutWindowOptions(), ...(bounds ?? {}) },
      };
    }
    if (decision === "external") void shell.openExternal(url);
    return { action: "deny" };
  });
  win.webContents.on("did-create-window", (child, details) => {
    lockPopoutWindow(child, details.url);
    const dock = workbenchPopoutDockId(details.url, trustContext);
    if (!dock) return;
    rememberPopout(dock, child);
    const { x, y, width, height } = details.options;
    const requested =
      typeof x === "number" &&
      typeof y === "number" &&
      typeof width === "number" &&
      typeof height === "number" &&
      width >= 1 &&
      height >= 1
        ? { x, y, width, height }
        : null;
    presentPopout(child, requested);
  });
  const guardNavigation = (event: Electron.Event, url: string) => {
    const context = trustContext;
    if (!context || !isAllowedNavigation(context.activeSurface, url, context)) {
      event.preventDefault();
      return;
    }
    // The shell is actually leaving this document. Drop guest pages first so
    // they cannot stay painted over the next surface.
    disposeBrowserHost();
  };
  win.webContents.on("will-navigate", guardNavigation);
  win.webContents.on("will-redirect", guardNavigation);
  win.on("close", () => {
    disposeBrowserHost();
  });

  if (content.kind === "hub") {
    await loadHub(win);
  } else {
    // Skeleton: no trusted surface while the backend is starting. The
    // workbench later goes through safeLoadURL into this same window.
    trustContext = null;
    await win.loadURL(SKELETON_URL);
  }
  return win;
}

/** Resolve a CLI/env workspace. Absence deliberately starts the local hub. */
function resolveBootWorkspace(): string | null {
  const fromEnv =
    process.env.LITECODE_WORKSPACE?.trim() ||
    process.argv.find((_a, i, arr) => arr[i - 1] === "--workspace") ||
    null;
  return fromEnv ? normalizeWorkspace(fromEnv) : null;
}

function requireWorkspaceDirectory(workspacePath: string): string {
  const workspace = normalizeWorkspace(workspacePath);
  try {
    if (!fs.statSync(workspace).isDirectory()) {
      throw new Error("not a directory");
    }
  } catch {
    throw new Error(`Workspace folder is unavailable: ${workspace}`);
  }
  return workspace;
}

function recordRecent(workspace: string): void {
  try {
    recents?.record(workspace);
  } catch (error) {
    console.error("Unable to persist recent workspace", error);
  }
}

function productVersion(): string {
  const pkgPath = path.join(__dirname, "..", "package.json");
  const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8")) as { version?: string };
  if (!pkg.version) throw new Error("desktop/package.json is missing version");
  return pkg.version;
}

function remoteBundleInstallDir(): string {
  return `.litecode/litecode-${productVersion()}`;
}

function devRepoRoot(): string | undefined {
  return devRepoRootFor(app.isPackaged, app.getAppPath());
}

let componentManagerInstance: ComponentManager | null = null;
const componentListeners = new Set<(event: ComponentProgressEvent) => void>();

function broadcastComponentProgress(event: ComponentProgressEvent): void {
  for (const listener of componentListeners) {
    try {
      listener(event);
    } catch (error) {
      console.warn("component progress listener failed", error);
    }
  }
  if (!mainWindow || mainWindow.isDestroyed()) return;
  mainWindow.webContents.send(COMPONENT_PROGRESS_CHANNEL, event);
}

/**
 * Slim installer: embed model + Linux server tar are fetched on first use from
 * this version's GitHub Release (or LITECODE_COMPONENTS_BASE). Lookup of an
 * existing local copy keeps the env → bundles → sidecar → repo order.
 */
function componentManager(): ComponentManager {
  if (!componentManagerInstance) {
    componentManagerInstance = new ComponentManager({
      appVersion: productVersion(),
      locate: {
        "embed-model": () =>
          resolveBundledModelDir({ sidecarRoot: sidecarRoot(), repoRoot: devRepoRoot() }),
        "linux-server": () => {
          try {
            return linuxBundlePaths().tar;
          } catch {
            return null;
          }
        },
      },
      emit: broadcastComponentProgress,
    });
  }
  return componentManagerInstance;
}

/** `ensure` a component while mirroring its download progress into the Open Remote bar. */
async function ensureComponentForRemote(
  id: ComponentId,
  range: [number, number],
): Promise<EnsureResult> {
  return ensureComponentForRemoteCore({
    id,
    range,
    ensure: (componentId) => componentManager().ensure(componentId as ComponentId),
    subscribe: (listener) => {
      componentListeners.add(listener);
    },
    unsubscribe: (listener) => {
      componentListeners.delete(listener);
    },
    emit: emitRemoteProgress,
  });
}

function linuxBundlePaths(): { tar: string; checksum: string } {
  const packaged = path.join(process.resourcesPath, "linux", LINUX_TAR_NAME);
  return resolveLinuxBundle({
    packagedTar: fs.existsSync(packaged) ? packaged : undefined,
    developmentTar: devLinuxBundleTar(app.isPackaged, app.getAppPath()),
  });
}

async function materializeSshTarget(target: SavedSshTarget): Promise<{
  target: SavedSshTarget;
  keyFile?: string;
  keyDirectory?: string;
  askPassCommand?: string;
}> {
  if (!target.credentialId) return { target };
  const credential = await getSecret(target.credentialId);
  if (!credential) throw new Error(t("desktop.error.credentialUnavailable"));
  if (target.credentialKind === "password") {
    const materialized = materializePasswordAskPass(credential);
    return { target, ...materialized };
  }
  const materialized = materializePrivateKeyFile(credential);
  return {
    target: { ...target, identityFile: materialized.keyFile },
    keyFile: materialized.keyFile,
    keyDirectory: materialized.keyDirectory,
  };
}

function emitRemoteProgress(event: RemoteProgressEvent): void {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  mainWindow.webContents.send(REMOTE_PROGRESS_CHANNEL, event);
}

type TrustedHost = { fingerprint: string; entry: string };

async function ensureSshHostTrusted(target: SavedSshTarget): Promise<string> {
  const userData = app.getPath("userData");
  const recordPath = path.join(userData, "ssh-host-keys.json");
  const knownHostsPath = path.join(userData, "ssh-known_hosts");
  const key = `${target.host}:${target.port ?? 22}`;
  let trusted: Record<string, TrustedHost> = {};
  try {
    trusted = JSON.parse(fs.readFileSync(recordPath, "utf8")) as Record<string, TrustedHost>;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") console.warn("Unable to read saved SSH host keys", error);
  }
  const discovered = await scanSshHostKey(target);
  const existing = trusted[key];
  if (existing && existing.fingerprint !== discovered.fingerprint) {
    throw new Error(`SSH host key changed for ${key}. Refusing the connection until the saved host record is deliberately removed.`);
  }
  if (!existing) {
    const options: MessageBoxOptions = {
      type: "warning",
      buttons: [t("desktop.dialog.hostTrust.trust"), t("desktop.dialog.hostTrust.cancel")],
      defaultId: 1,
      cancelId: 1,
      title: t("desktop.dialog.hostTrust.title"),
      message: t("desktop.dialog.hostTrust.firstConnection", { key }),
      detail: t("desktop.dialog.hostTrust.detail", { fingerprint: discovered.fingerprint }),
    };
    const result = mainWindow
      ? await dialog.showMessageBox(mainWindow, options)
      : await dialog.showMessageBox(options);
    if (result.response !== 0) throw new Error(t("desktop.error.hostNotApproved"));
    trusted[key] = discovered;
    fs.mkdirSync(userData, { recursive: true });
    fs.writeFileSync(recordPath, `${JSON.stringify(trusted, null, 2)}\n`, { encoding: "utf8", mode: 0o600 });
    fs.appendFileSync(knownHostsPath, `${discovered.entry}\n`, { encoding: "utf8", mode: 0o600 });
  }
  return knownHostsPath;
}

async function unusedLoopbackPort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("Could not reserve a local tunnel port."));
        return;
      }
      server.close((error) => error ? reject(error) : resolve(address.port));
    });
  });
}

type HealthPayload = {
  ok?: boolean;
  workspace_root?: string;
  workspace_id?: string;
};

function normalizeHealthRoot(root: string): string {
  return normalizeWorkspace(root);
}

/** POSIX compare for remote Linux workspace roots (avoid Windows path.resolve). */
function posixHealthRoot(root: string): string {
  return root.replace(/\\/g, "/").replace(/\/+$/, "") || "/";
}

function healthRootsMatch(reported: string, expected: string): boolean {
  if (expected.startsWith("/")) {
    return posixHealthRoot(reported) === posixHealthRoot(expected);
  }
  return normalizeHealthRoot(reported) === normalizeHealthRoot(expected);
}

function healthUrlFromBase(baseUrl: string): string {
  const trimmed = baseUrl.trim().replace(/\/$/, "");
  return `${trimmed}/health`;
}

/**
 * Wait until /health reports a bound workspace identity (WorkspaceVerified).
 * When `expectedRoot` is set, the reported root must match (LAP compare).
 */
async function waitForWorkspaceHealth(
  healthUrl: string,
  expectedRoot?: string,
): Promise<HealthPayload> {
  const deadline = Date.now() + 15_000;
  let lastError = "Runtime did not become WorkspaceVerified.";
  while (Date.now() < deadline) {
    try {
      const response = await fetch(healthUrl);
      if (!response.ok) {
        lastError = `Health check returned HTTP ${response.status}.`;
      } else {
        const body = (await response.json()) as HealthPayload;
        if (!body.ok) {
          lastError = "Health reported ok=false.";
        } else if (!body.workspace_id || !body.workspace_root) {
          lastError =
            "Health missing workspace_id/workspace_root (not WorkspaceVerified).";
        } else if (
          expectedRoot &&
          !healthRootsMatch(body.workspace_root, expectedRoot)
        ) {
          lastError = `Workspace root mismatch: expected ${expectedRoot}, got ${body.workspace_root}.`;
        } else {
          return body;
        }
      }
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Runtime did not become ready: ${lastError}`);
}

async function waitForRemoteHealth(port: number, expectedRoot?: string): Promise<HealthPayload> {
  return waitForWorkspaceHealth(`http://127.0.0.1:${port}/health`, expectedRoot);
}

async function verifySidecarAttached(handle: SidecarHandle): Promise<HealthPayload> {
  const port = new URL(handle.readyUrl).port;
  if (!port) {
    throw new Error("Sidecar READY URL is missing a port.");
  }
  return waitForRemoteHealth(Number(port), handle.workspace);
}

function cancelTunnelReconnect(): void {
  if (tunnelReconnectTimer) {
    clearTimeout(tunnelReconnectTimer);
    tunnelReconnectTimer = null;
  }
}

function cancelSidecarRestart(): void {
  if (sidecarRestartTimer) {
    clearTimeout(sidecarRestartTimer);
    sidecarRestartTimer = null;
  }
}

function emitRemoteDisconnected(reason: string): void {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  try {
    mainWindow.webContents.send(REMOTE_DISCONNECTED_CHANNEL, { reason });
  } catch (error) {
    console.warn("Failed to emit remote-disconnected", error);
  }
}

async function stopManagedRemote(): Promise<void> {
  cancelTunnelReconnect();
  tunnelRecovery.reset();
  const active = managedRemote;
  managedRemote = null;
  if (!active) return;
  active.tunnel.markIntentionalStop();
  active.session.stopTunnel(active.tunnel);
  await active.session.stopServe(active.serve).catch((error) => {
    console.warn("Failed to stop managed remote workspace", error);
  });
  removeMaterializedKey(active.keyDirectory);
}

function scheduleTunnelReconnect(targetId: string, attempt: number): void {
  cancelTunnelReconnect();
  const delay = recoveryDelayMs(attempt, DEFAULT_TUNNEL_RECOVERY);
  tunnelReconnectTimer = setTimeout(() => {
    tunnelReconnectTimer = null;
    void (async () => {
      try {
        console.warn(`[litecode] auto-reconnecting remote tunnel (attempt ${attempt})…`);
        await reconnectRemote(targetId);
        tunnelRecovery.reset();
      } catch (error) {
        console.warn("[litecode] tunnel auto-reconnect failed", error);
        emitRemoteDisconnected(
          error instanceof Error ? error.message : "Tunnel reconnect failed.",
        );
      }
    })();
  }, delay);
}

function onTunnelUnexpectedExit(info: TunnelExitInfo, binding: ManagedRemote): void {
  if (info.intentional || quitting) return;
  // Only act if this tunnel is still the active managed remote.
  if (managedRemote?.tunnel !== binding.tunnel) return;
  console.warn(
    `[litecode] SSH tunnel exited unexpectedly (code=${info.code} signal=${info.signal})`,
  );
  const targetId = binding.targetId;
  const workspace = binding.workspace;
  managedRemote = null;
  void binding.session.stopServe(binding.serve).catch(() => undefined);
  removeMaterializedKey(binding.keyDirectory);
  emitRemoteDisconnected("SSH tunnel disconnected.");
  if (!targetId || !workspace) return;
  if (!tunnelRecovery.recordUnexpected(DEFAULT_TUNNEL_RECOVERY)) {
    console.warn("[litecode] tunnel auto-reconnect exhausted");
    return;
  }
  scheduleTunnelReconnect(targetId, tunnelRecovery.count);
}

function attachTunnelWatch(binding: ManagedRemote): void {
  const child = binding.tunnel.process;
  // startTunnel already attached onExit once; also listen here in case options
  // were omitted (tests / older callers). Use a dedicated once per binding.
  const onExit = (code: number | null, signal: NodeJS.Signals | null) => {
    onTunnelUnexpectedExit(
      { code, signal, intentional: binding.tunnel.intentionalStop },
      binding,
    );
  };
  child.prependOnceListener("exit", onExit);
}

function attachSidecarWatch(handle: SidecarHandle): void {
  watchSidecarExit(handle, (info) => {
    if (info.intentional || quitting) return;
    if (sidecar !== handle) return;
    console.warn(
      `[litecode] sidecar exited after READY (code=${info.code} signal=${info.signal})`,
    );
    sidecar = null;
    syncDevUpstream(null);
    if (sessionMode !== "local" || !currentWorkspace) {
      emitRemoteDisconnected("Local sidecar crashed.");
      return;
    }
    if (!sidecarRecovery.recordUnexpected(DEFAULT_SIDECAR_RECOVERY)) {
      console.warn("[litecode] sidecar auto-restart exhausted");
      if (mainWindow && !mainWindow.isDestroyed()) {
        void dialog.showMessageBox(mainWindow, {
          type: "error",
          title: t("desktop.dialog.sidecarCrash.title"),
          message: t("desktop.dialog.sidecarCrash.message"),
          detail: `code=${info.code ?? "null"} signal=${info.signal ?? "null"}`,
        });
      }
      return;
    }
    const workspace = currentWorkspace;
    const attempt = sidecarRecovery.count;
    const delay = recoveryDelayMs(attempt, DEFAULT_SIDECAR_RECOVERY);
    cancelSidecarRestart();
    sidecarRestartTimer = setTimeout(() => {
      sidecarRestartTimer = null;
      void sidecarLifecycle
        .run(async () => {
          console.warn(`[litecode] auto-restarting sidecar (attempt ${attempt})…`);
          authToken = authToken || createToken();
          const next = await startSidecar({
            token: authToken,
            parentPid: process.pid,
            workspace,
          });
          await verifySidecarAttached(next);
          sidecar = next;
          attachSidecarWatch(next);
          sidecarRecovery.reset();
          if (mainWindow && !mainWindow.isDestroyed()) {
            await loadLocalWorkbench(mainWindow, next.readyUrl);
          }
        })
        .catch((error) => {
          console.warn("[litecode] sidecar auto-restart failed", error);
        });
    }, delay);
  });
}

async function installLinuxBundle(session: SshSession): Promise<void> {
  const dest = remoteBundleInstallDir();
  let modelDir = resolveBundledModelDir({
    sidecarRoot: sidecarRoot(),
    repoRoot: devRepoRoot(),
  });
  if (!modelDir) {
    // Best effort: remote code search needs it, Open Remote itself does not.
    try {
      modelDir = (await ensureComponentForRemote("embed-model", [0.06, 0.12])).path;
    } catch (error) {
      console.warn("Embed model download failed; continuing Open Remote without it", error);
      emitRemoteProgress({
        stage: "download",
        ratio: 0.12,
        message: t("desktop.progress.codeSearchUnavailable"),
      });
    }
  }
  if (modelDir) {
    emitRemoteProgress({
      stage: "upload",
      ratio: 0.12,
      message: t("desktop.progress.uploadingEmbedModel"),
    });
    await session.installModelDir({
      localModelDir: modelDir,
      destination: dest,
      relativeModelPath: `models/${BUNDLED_MODEL_REL}`,
    });
  } else {
    console.warn("Embed model not found at convention paths; remote code search will be unavailable until models are synced.");
  }

  const linux = await ensureComponentForRemote("linux-server", [0.14, 0.38]);
  const tar = linux.path;
  const checksum = `${tar}.sha256`;
  const hash = fs.readFileSync(checksum, "utf8").trim().split(/\s+/)[0];
  if (!hash) throw new Error("Linux server checksum file is invalid.");
  await session.installTar({
    localTarPath: tar,
    sha256: hash,
    destination: dest,
    onProgress: (progress) => {
      emitRemoteProgress({
        stage: progress.stage,
        ratio: 0.4 + progress.ratio * 0.45,
        message: progress.message,
      });
    },
  });
}

async function startRemoteServeAndTunnel(
  session: SshSession,
  home: string,
  workspace: string,
): Promise<{ serve: RemoteServeHandle; tunnel: SshTunnelHandle; token: string; localPort: number }> {
  emitRemoteProgress({ stage: "starting", ratio: 0.9, message: t("desktop.progress.startingRemoteServe") });
  const remotePort = await unusedLoopbackPort();
  const localPort = await unusedLoopbackPort();
  const token = createToken();
  const serve = await session.startServe({
    port: remotePort,
    token,
    workspace,
    executable: `${home}/${remoteBundleInstallDir()}/litecode`,
  });
  // Exit listener is attached after managedRemote is bound (see bindManagedRemote).
  const tunnel = session.startTunnel(localPort, remotePort);
  const expectedRoot = `${home.replace(/\/$/, "")}/${workspace.replace(/^\//, "")}`;
  try {
    await waitForRemoteHealth(localPort, expectedRoot);
  } catch (error) {
    session.stopTunnel(tunnel);
    await session.stopServe(serve).catch(() => undefined);
    const log = await session.readSessionLog(serve.id).catch(() => "");
    const base = error instanceof Error ? error.message : String(error);
    throw new Error(log ? `${base}\n\nRemote serve log (tail):\n${log}` : base);
  }
  return { serve, tunnel, token, localPort };
}

function bindManagedRemote(
  partial: Omit<ManagedRemote, "tunnel"> & { tunnel: SshTunnelHandle },
): ManagedRemote {
  const binding: ManagedRemote = { ...partial };
  managedRemote = binding;
  attachTunnelWatch(binding);
  tunnelRecovery.reset();
  return binding;
}

async function connectManagedSsh(targetId: string, workspace: string): Promise<{ ok: true; mode: "remote" }> {
  const target = sshTargets?.get(targetId);
  if (!target) throw new Error(t("desktop.error.sshTargetNotFound"));
  emitRemoteProgress({ stage: "authenticating", ratio: 0.05, message: t("desktop.progress.connectingSsh") });
  const knownHostsFile = await ensureSshHostTrusted(target);
  const materialized = await materializeSshTarget(target);
  const session = new SshSession({
    target: materialized.target,
    askPassCommand: materialized.askPassCommand,
    knownHostsFile,
  });
  let serve: RemoteServeHandle | null = null;
  let tunnel: SshTunnelHandle | null = null;
  try {
    const { home } = await session.probeRemoteHome();
    await installLinuxBundle(session);
    const started = await startRemoteServeAndTunnel(session, home, workspace);
    serve = started.serve;
    tunnel = started.tunnel;
    await stopManagedRemote();
    // D8: bind only after connectRemote succeeds so a failed attach cannot
    // leave managedRemote pointing at a stopped tunnel/serve.
    sshTargets?.updateConnection(target.id, workspace);
    emitRemoteProgress({ stage: "attaching", ratio: 1, message: t("desktop.progress.openingWorkspace") });
    const result = await connectRemote(`http://127.0.0.1:${started.localPort}`, started.token);
    bindManagedRemote({
      session,
      serve,
      tunnel,
      keyFile: materialized.keyFile,
      keyDirectory: materialized.keyDirectory,
      targetId: target.id,
      workspace,
    });
    return result;
  } catch (error) {
    session.stopTunnel(tunnel);
    await session.stopServe(serve).catch(() => undefined);
    removeMaterializedKey(materialized.keyDirectory);
    managedRemote = clearManagedRemoteIfSession(managedRemote, session);
    throw error;
  }
}

type StartRemoteInput = {
  userAtHost: string;
  password?: string;
  authMode?: "password" | "private_key" | "agent";
  identityFile?: string;
  label?: string;
};

async function startRemoteSession(input: StartRemoteInput): Promise<{
  sessionId: string;
  home: string;
  label: string;
}> {
  return openRemoteStartFlight.run(() => startRemoteSessionBody(input));
}

async function startRemoteSessionBody(input: StartRemoteInput): Promise<{
  sessionId: string;
  home: string;
  label: string;
}> {
  const parsed = parseUserAtHost(input.userAtHost);
  const authMode = input.authMode ?? (input.password ? "password" : input.identityFile ? "private_key" : "agent");
  if (authMode === "password" && !input.password?.trim()) {
    throw new Error(t("desktop.error.passwordRequired"));
  }
  if (authMode === "private_key" && !input.identityFile?.trim()) {
    throw new Error(t("desktop.error.privateKeyRequired"));
  }

  emitRemoteProgress({ stage: "authenticating", ratio: 0.05, message: t("desktop.progress.verifyingSsh") });

  const draft: SavedSshTarget = {
    id: createPendingId(),
    label: input.label?.trim() || formatUserAtHost(parsed),
    host: parsed.host,
    ...(parsed.user ? { user: parsed.user } : {}),
    ...(parsed.port ? { port: parsed.port } : {}),
  };

  let keyDirectory: string | undefined;
  let askPassCommand: string | undefined;
  let identityFile: string | undefined;
  let credentialKind: PendingRemoteSession["credentialKind"] = authMode;
  let credentialMaterial: string | undefined;

  try {
    if (authMode === "password") {
      credentialMaterial = input.password!.trim();
      const materialized = materializePasswordAskPass(credentialMaterial);
      keyDirectory = materialized.keyDirectory;
      askPassCommand = materialized.askPassCommand;
    } else if (authMode === "private_key") {
      credentialMaterial = readPrivateKeyFile(String(input.identityFile));
      const materialized = materializePrivateKeyFile(credentialMaterial);
      keyDirectory = materialized.keyDirectory;
      identityFile = materialized.keyFile;
    }

    const knownHostsFile = await ensureSshHostTrusted(draft);
    const session = new SshSession({
      target: {
        host: draft.host,
        ...(draft.user ? { user: draft.user } : {}),
        ...(draft.port ? { port: draft.port } : {}),
        ...(identityFile ? { identityFile } : {}),
      },
      askPassCommand,
      knownHostsFile,
    });

    const { home } = await session.probeRemoteHome();
    await installLinuxBundle(session);

    const pending: PendingRemoteSession = {
      id: draft.id,
      label: draft.label,
      host: draft.host,
      ...(draft.user ? { user: draft.user } : {}),
      ...(draft.port ? { port: draft.port } : {}),
      credentialKind,
      ...(identityFile ? { identityFile } : {}),
      home,
      session,
      keyDirectory,
      knownHostsFile,
      ...(credentialMaterial ? { credentialMaterial } : {}),
    };
    setPendingRemote(pending);
    emitRemoteProgress({ stage: "ready", ratio: 0.88, message: t("desktop.progress.connected") });
    // Ownership of the temp key dir now lives in the pending session (cleaned
    // on cancel/complete/stale-clear); clear it so finally does not remove it.
    keyDirectory = undefined;
    return { sessionId: pending.id, home, label: pending.label };
  } finally {
    // DESK-03: remove the plaintext password / key temp dir on any thrown
    // path so a failed start leaves no secret material on disk.
    removeMaterializedKey(keyDirectory);
  }
}

async function listPendingRemoteDirs(
  sessionId: string,
  relativePath = ".",
): Promise<{ path: string; home: string; entries: Array<{ name: string }> }> {
  const pending = getPendingRemote(sessionId);
  const dir = String(relativePath ?? ".").trim() || ".";
  const entries = await pending.session.listDirectory(dir);
  return { path: dir, home: pending.home, entries };
}

async function completeRemoteSession(
  sessionId: string,
  workspace: string,
): Promise<{ token: string; baseUrl: string; workspace: string; label: string }> {
  return openRemoteCompleteFlight.run(() => completeRemoteSessionBody(sessionId, workspace));
}

async function completeRemoteSessionBody(
  sessionId: string,
  workspace: string,
): Promise<{ token: string; baseUrl: string; workspace: string; label: string }> {
  const pending = getPendingRemote(sessionId);
  const relative = String(workspace ?? "").trim() || ".";
  if (relative === ".") {
    throw new Error(t("hub.status.pickFolderInHome"));
  }

  // Idempotent: a double-complete reuses the already-started serve (D4).
  if (pending.ready) {
    return {
      token: pending.ready.token,
      baseUrl: pending.ready.baseUrl,
      workspace: pending.ready.workspace,
      label: pending.label,
    };
  }

  const started = await startRemoteServeAndTunnel(pending.session, pending.home, relative);
  await stopManagedRemote();
  bindManagedRemote({
    session: pending.session,
    serve: started.serve,
    tunnel: started.tunnel,
    keyDirectory: pending.keyDirectory,
    targetId: pending.id,
    workspace: relative,
  });

  const baseUrl = `http://127.0.0.1:${started.localPort}`;
  pending.ready = { token: started.token, baseUrl, workspace: relative };
  setPendingRemote(pending);

  emitRemoteProgress({
    stage: "ready",
    ratio: 0.96,
    // A4: transient — the hub enters the workspace right after this.
    message: t("desktop.progress.sessionReady"),
  });

  return {
    token: started.token,
    baseUrl,
    workspace: relative,
    label: pending.label,
  };
}

async function enterRemoteWorkbench(sessionId: string): Promise<{ ok: true; mode: "remote" }> {
  const pending = getPendingRemote(sessionId);
  if (!pending.ready) {
    throw new Error(t("desktop.error.sessionNotReady"));
  }
  emitRemoteProgress({ stage: "attaching", ratio: 1, message: t("desktop.progress.openingWorkspace") });
  let result: { ok: true; mode: "remote" };
  try {
    result = await connectRemote(pending.ready.baseUrl, pending.ready.token);
  } catch (error) {
    managedRemote = clearManagedRemoteIfSession(managedRemote, pending.session);
    throw error;
  }
  takePendingRemote(sessionId);

  const store = sshTargets;
  if (store) {
    const credentialMaterial = pending.credentialMaterial;
    let credentialId: string | undefined;
    let credentialKind: SavedSshTarget["credentialKind"];
    if (pending.credentialKind === "password" && credentialMaterial) {
      credentialKind = "password";
      credentialId = `ssh-password:${pending.id}`;
      await setSecret(credentialId, credentialMaterial);
    } else if (pending.credentialKind === "private_key" && credentialMaterial) {
      credentialKind = "private_key";
      credentialId = `ssh-private_key:${pending.id}`;
      await setSecret(credentialId, credentialMaterial);
    }
    store.save({
      id: pending.id,
      label: pending.label,
      host: pending.host,
      ...(pending.user ? { user: pending.user } : {}),
      ...(pending.port ? { port: pending.port } : {}),
      ...(credentialId ? { credentialId } : {}),
      ...(credentialKind ? { credentialKind } : {}),
      lastWorkspace: pending.ready.workspace,
    });
  }

  return result;
}

async function cancelRemoteSession(sessionId: string): Promise<void> {
  const pending = deletePendingRemote(sessionId);
  if (!pending) return;
  if (managedRemote?.session === pending.session) {
    await stopManagedRemote();
  }
  removeMaterializedKey(pending.keyDirectory);
}

async function reconnectRemote(targetId: string): Promise<{ ok: true; mode: "remote" }> {
  const target = sshTargets?.get(String(targetId));
  if (!target?.lastWorkspace) {
    throw new Error(t("desktop.error.remoteNeverConnected"));
  }
  return connectManagedSsh(target.id, target.lastWorkspace);
}

const DEV_UI_WAIT_MS = 30_000;

/**
 * Local workbench document. With `LITECODE_UI_DEV_URL` this is the Vite origin
 * (hot reload); otherwise it is the sidecar's own static page. The sidecar
 * port is published before the window loads so the dev proxy can reach it.
 */
async function localWorkbenchUrl(readyUrl: string): Promise<string> {
  const ui = devUiDocumentUrl(process.env.LITECODE_UI_DEV_URL);
  assertDevUiConfigured(ui, devUpstreamFilePath());
  syncDevUpstream(readyUrl);
  if (!ui) return readyUrl;
  await waitForDevUi(ui);
  console.log(`[litecode] workbench hot reload ${ui} → ${readyUrl}`);
  return ui;
}

async function waitForDevUi(url: string): Promise<void> {
  const deadline = Date.now() + DEV_UI_WAIT_MS;
  let lastError = "no response";
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`Vite dev server did not become ready at ${url}: ${lastError}`);
}

/**
 * Load the local workbench. A workspace switch keeps the same Vite document,
 * so `loadURL` of that URL is not enough — reload after the upstream file moves
 * to the new sidecar port.
 */
async function loadLocalWorkbench(win: BrowserWindow, readyUrl: string): Promise<void> {
  const target = await localWorkbenchUrl(readyUrl);
  let current = "";
  try {
    current = win.webContents.getURL();
  } catch {
    current = "";
  }
  if (sameWorkbenchDocument(current, target)) {
    await reloadWorkbench(win);
    return;
  }
  await safeLoadURL(win, target);
}

function reloadWorkbench(win: BrowserWindow): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (!win.isDestroyed()) {
        win.webContents.removeListener("did-finish-load", onLoad);
        win.webContents.removeListener("did-fail-load", onFail);
      }
      if (error) reject(error);
      else resolve();
    };
    const timer = setTimeout(() => {
      finish(new Error("Timed out reloading the workbench"));
    }, DEV_UI_WAIT_MS);
    const onLoad = () => finish();
    // -3 is ERR_ABORTED: the reload itself cancels the previous navigation.
    const onFail = (
      _event: Electron.Event,
      errorCode: number,
      errorDescription: string,
      _validatedURL: string,
      isMainFrame: boolean,
    ) => {
      if (!isMainFrame || errorCode === -3) return;
      finish(new Error(`Reload failed (${errorCode}): ${errorDescription}`));
    };
    win.webContents.once("did-finish-load", onLoad);
    win.webContents.on("did-fail-load", onFail);
    try {
      win.webContents.reloadIgnoringCache();
    } catch (error) {
      finish(error instanceof Error ? error : new Error(String(error)));
    }
  });
}

/**
 * Stop current sidecar (if any) and start a new one rooted at `workspace`.
 * Reloads the main window. The sidecar binds a new ephemeral port; in the Vite
 * dev loop the document URL stays put and the proxy follows the new port.
 */
async function relaunchSidecar(workspacePath: string): Promise<{ ok: true; project: string }> {
  const workspace = requireWorkspaceDirectory(workspacePath);
  return sidecarLifecycle.run(async () => {
    cancelSidecarRestart();
    sidecarRecovery.reset();
    await stopSidecar(sidecar);
    sidecar = null;
    syncDevUpstream(null);

    sessionMode = "local";
    const next = await startSidecar({
      token: authToken,
      parentPid: process.pid,
      workspace,
    });
    await verifySidecarAttached(next);
    sidecar = next;
    attachSidecarWatch(next);
    currentWorkspace = workspace;
    registry?.setWorkspace(workspace);
    recordRecent(workspace);

    if (mainWindow && !mainWindow.isDestroyed()) {
      await loadLocalWorkbench(mainWindow, sidecar.readyUrl);
    }
    return { ok: true, project: workspace };
  });
}

async function connectRemote(baseUrl: string, token: string): Promise<{ ok: true; mode: "remote" }> {
  const url = handshakeUrl(baseUrl, token);
  await waitForWorkspaceHealth(healthUrlFromBase(baseUrl));
  return sidecarLifecycle.run(async () => {
    cancelSidecarRestart();
    sidecarRecovery.reset();
    await stopSidecar(sidecar);
    sidecar = null;
    syncDevUpstream(null);
    sessionMode = "remote";
    authToken = token;
    currentWorkspace = null;
    registry?.setWorkspace(null);

    if (mainWindow && !mainWindow.isDestroyed()) {
      await safeLoadURL(mainWindow, url);
    }
    return { ok: true, mode: "remote" } as const;
  });
}

async function connectSavedRemote(targetId: string): Promise<{ ok: true; mode: "remote" }> {
  const target = remoteTargets?.get(targetId);
  if (!target) throw new Error("Remote server target was not found.");
  const token = await getSecret(target.credentialId);
  if (!token) throw new Error("The saved token for this remote server is unavailable.");
  return connectRemote(target.baseUrl, token);
}

async function boot(): Promise<void> {
  // Formal remote + legacy DEV escape hatch (same attach semantics).
  // `LITECODE_UI_DEV_URL` is not that hatch: it only swaps the local document
  // for Vite. Parse it first so a bad value fails before the sidecar starts.
  const devUi = devUiDocumentUrl(process.env.LITECODE_UI_DEV_URL);
  const remoteUrl =
    process.env.LITECODE_REMOTE_URL?.trim() ||
    process.env.LITECODE_DEV_URL?.trim() ||
    "";
  if (!remoteUrl) {
    assertDevUiConfigured(devUi, devUpstreamFilePath());
  }
  if (remoteUrl) {
    const token = process.env.LITECODE_TOKEN?.trim();
    if (!token) {
      throw new Error(
        "Remote attach requires LITECODE_TOKEN (set the same token the remote serve uses).",
      );
    }
    sessionMode = "remote";
    authToken = token;
    const expected = process.env.LITECODE_WORKSPACE?.trim();
    const expectedWorkspace = expected ? normalizeWorkspace(expected) : null;
    // S1b: skeleton first; /health only gates loadURL(handshakeUrl).
    await runBootSequence<BrowserWindow, null>({
      startBackend: async () => {
        await waitForWorkspaceHealth(healthUrlFromBase(remoteUrl), expectedWorkspace ?? undefined);
        return null;
      },
      createSkeleton: () => createBootSkeleton(expectedWorkspace),
      prepare: async () => {
        if (expectedWorkspace) currentWorkspace = expectedWorkspace;
        return handshakeUrl(remoteUrl, authToken);
      },
      loadWorkbench: (win, url) => safeLoadURL(win, url),
      stopBackend: async () => undefined,
      closeSkeleton: closeFailedBootSkeleton,
      // No local process to stop; quit must not wait for the health poll.
    });
    return;
  }

  sessionMode = "local";
  authToken = createToken();
  const workspace = resolveBootWorkspace();
  if (!workspace) {
    const hubWindow = await createWindow({ kind: "hub" });
    mainWindow = hubWindow;
    hubWindow.on("closed", () => {
      if (mainWindow === hubWindow) mainWindow = null;
    });
    await startRegistry(hubWindow, null);
    return;
  }
  // S1: skeleton window in parallel with the sidecar; READY + attach check +
  // /health only gate the workbench loadURL into the same window.
  await runBootSequence<BrowserWindow, SidecarHandle>({
    startBackend: (signal) =>
      startSidecar({
        token: authToken,
        parentPid: process.pid,
        workspace,
        // Window closed during the skeleton: kill the child now, not at READY.
        signal,
      }),
    createSkeleton: () => createBootSkeleton(workspace),
    onBackend: (handle) => {
      sidecar = handle;
    },
    prepare: async (handle) => {
      await verifySidecarAttached(handle);
      attachSidecarWatch(handle);
      currentWorkspace = workspace;
      return localWorkbenchUrl(handle.readyUrl);
    },
    loadWorkbench: (win, url) => safeLoadURL(win, url),
    stopBackend: async (handle) => {
      await stopSidecar(handle);
      if (sidecar === handle) {
        sidecar = null;
        syncDevUpstream(null);
      }
    },
    closeSkeleton: closeFailedBootSkeleton,
    // S2: boot-time recents write only after the workbench loaded.
    recordRecent: () => recordRecent(workspace),
    onAbort: (cleanup) => {
      bootAbortCleanup = cleanup;
    },
  });
}

/** Set while a failed boot closes its skeleton, so that close does not quit the app. */
let closingFailedBoot = false;
/** Abort cleanup (sidecar stop) when the user closed the skeleton; awaited before exit. */
let bootAbortCleanup: Promise<void> | null = null;

async function startRegistry(win: BrowserWindow, workspace: string | null): Promise<void> {
  registry = new InstanceRegistry();
  await registry.start(win);
  if (workspace) {
    registry.setWorkspace(workspace);
  }
}

/** Skeleton window + registry as soon as the workspace is known (second open focuses it). */
async function createBootSkeleton(workspace: string | null): Promise<BrowserWindow> {
  const win = await createWindow({ kind: "skeleton" });
  mainWindow = win;
  win.on("closed", () => {
    if (mainWindow === win) mainWindow = null;
  });
  await startRegistry(win, workspace);
  return win;
}

function closeFailedBootSkeleton(win: BrowserWindow): void {
  closingFailedBoot = true;
  win.destroy();
}

function targetWindow(
  event: Electron.IpcMainEvent | Electron.IpcMainInvokeEvent,
): BrowserWindow | null {
  return senderOwnedWindow(
    event.sender,
    (sender) => BrowserWindow.fromWebContents(sender),
  ) as BrowserWindow | null;
}

function requireTrustedIpc(
  event: Electron.IpcMainEvent | Electron.IpcMainInvokeEvent,
  allowed: AllowedSurface,
): void {
  const context = trustContext;
  const win = targetWindow(event);
  if (!context || !mainWindow || win !== mainWindow) {
    throw new Error("Rejected IPC without trusted window ownership");
  }
  assertIpcSurface(event, mainWindow.webContents, context, allowed);
}

function onTrusted(
  channel: string,
  allowed: AllowedSurface,
  listener: (event: Electron.IpcMainEvent, ...args: any[]) => void,
): void {
  ipcMain.on(channel, (event, ...args) => {
    try {
      requireTrustedIpc(event, allowed);
      listener(event, ...args);
    } catch {
      // Synchronous IPC cannot return a rejected Promise. Return no capability
      // data without letting an untrusted sendSync become a main-process DoS.
      event.returnValue = undefined;
    }
  });
}

function handleTrusted(
  channel: string,
  allowed: AllowedSurface,
  listener: (event: Electron.IpcMainInvokeEvent, ...args: any[]) => any,
): void {
  ipcMain.handle(channel, (event, ...args) => {
    requireTrustedIpc(event, allowed);
    return listener(event, ...args);
  });
}

function registerNativeViewIpc(): void {
  handleTrusted(NATIVE_VIEW_GET_SNAPSHOT_CHANNEL, "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    const kind = body.kind === "app" ? "app" : body.kind === "browser" ? "browser" : null;
    const id = typeof body.id === "string" ? body.id : "";
    if (!kind || !id) return "";
    const host = kind === "browser" ? browserHost : appViewHost;
    return host ? host.getSnapshot(id) : "";
  });

  onTrusted(NATIVE_VIEW_SET_OCCLUDED_CHANNEL, "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    const kind = body.kind === "app" ? "app" : body.kind === "browser" ? "browser" : null;
    const id = typeof body.id === "string" ? body.id : "";
    if (!kind || !id) return;
    const host = kind === "browser" ? browserHost : appViewHost;
    host?.setViewOccluded(id, body.occluded === true);
  });
}

function registerBrowserIpc(): void {
  handleTrusted("litecode:browser-create", "workbench", (_event, payload: unknown) => {
    const host = ensureBrowserHost();
    if (!host) throw new Error("Browser is unavailable");
    const body = asRecord(payload);
    return host.create(typeof body.id === "string" ? body.id : "", body.backgroundColor);
  });

  handleTrusted("litecode:browser-navigate", "workbench", (_event, payload: unknown) => {
    const host = ensureBrowserHost();
    if (!host) throw new Error("Browser is unavailable");
    const body = asRecord(payload);
    return host.navigate(typeof body.id === "string" ? body.id : "", body.url);
  });

  handleTrusted("litecode:browser-go-back", "workbench", (_event, id: unknown) => {
    const host = ensureBrowserHost();
    if (!host) throw new Error("Browser is unavailable");
    return host.goBack(typeof id === "string" ? id : "");
  });

  handleTrusted("litecode:browser-go-forward", "workbench", (_event, id: unknown) => {
    const host = ensureBrowserHost();
    if (!host) throw new Error("Browser is unavailable");
    return host.goForward(typeof id === "string" ? id : "");
  });

  handleTrusted("litecode:browser-reload", "workbench", (_event, id: unknown) => {
    const host = ensureBrowserHost();
    if (!host) throw new Error("Browser is unavailable");
    return host.reload(typeof id === "string" ? id : "");
  });

  handleTrusted("litecode:browser-stop", "workbench", (_event, id: unknown) => {
    const host = ensureBrowserHost();
    if (!host) throw new Error("Browser is unavailable");
    return host.stop(typeof id === "string" ? id : "");
  });

  onTrusted("litecode:browser-set-bounds", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    browserHost?.setBounds(body.id, body.bounds, body.place);
  });

  onTrusted("litecode:browser-set-host", "workbench", (event, payload: unknown) => {
    const body = asRecord(payload);
    const host = ensureBrowserHost();
    event.returnValue = host ? host.setHost(body.id, body.popoutId ?? null) : false;
  });

  onTrusted("litecode:browser-set-visible", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    browserHost?.setVisible(body.id, body.visible === true);
  });

  onTrusted("litecode:browser-set-active", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    browserHost?.setActive(body.id, body.active === true);
  });

  onTrusted("litecode:browser-destroy", "workbench", (_event, id: unknown) => {
    browserHost?.destroy(id);
  });

  onTrusted("litecode:browser-set-obscured", "workbench", (event, obscured: unknown) => {
    browserObscured = obscured === true;
    browserHost?.setObscured(browserObscured);
    appViewHost?.setObscured(browserObscured);
    event.returnValue = undefined;
  });
}

function registerAppViewIpc(): void {
  handleTrusted("litecode:app-view-create", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    const id = typeof body.id === "string" ? body.id : "";
    const viewKey = typeof body.viewKey === "string" ? body.viewKey : "";
    const url = typeof body.url === "string" ? body.url : "";
    if (!isLoopbackHttpUrl(url)) throw new Error("App view URL must be loopback http");
    const origin = httpOrigin(url);
    if (!origin) throw new Error("App view URL is invalid");
    const partition = appViewPartition(viewKey);
    const permissions = Array.isArray(body.permissions)
      ? body.permissions.filter((value): value is string => typeof value === "string")
      : [];
    configureAppSession(partition, permissions);
    appViewPartitions.set(id, partition);
    appViewOrigins.set(id, origin);
    const host = ensureAppViewHost();
    if (!host) throw new Error("App view is unavailable");
    try {
      host.create(id, body.backgroundColor);
      return host.navigate(id, url);
    } catch (error) {
      host.destroy(id);
      appViewPartitions.delete(id);
      appViewOrigins.delete(id);
      throw error;
    }
  });

  handleTrusted("litecode:app-view-go-back", "workbench", (_event, id: unknown) => {
    const host = ensureAppViewHost();
    if (!host) throw new Error("App view is unavailable");
    return host.goBack(typeof id === "string" ? id : "");
  });

  handleTrusted("litecode:app-view-go-forward", "workbench", (_event, id: unknown) => {
    const host = ensureAppViewHost();
    if (!host) throw new Error("App view is unavailable");
    return host.goForward(typeof id === "string" ? id : "");
  });

  handleTrusted("litecode:app-view-reload", "workbench", (_event, id: unknown) => {
    const host = ensureAppViewHost();
    if (!host) throw new Error("App view is unavailable");
    return host.reload(typeof id === "string" ? id : "");
  });

  handleTrusted("litecode:app-view-stop", "workbench", (_event, id: unknown) => {
    const host = ensureAppViewHost();
    if (!host) throw new Error("App view is unavailable");
    return host.stop(typeof id === "string" ? id : "");
  });

  onTrusted("litecode:app-view-set-bounds", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    appViewHost?.setBounds(body.id, body.bounds, "main");
  });

  onTrusted("litecode:app-view-set-visible", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    appViewHost?.setVisible(body.id, body.visible === true);
  });

  onTrusted("litecode:app-view-set-active", "workbench", (_event, payload: unknown) => {
    const body = asRecord(payload);
    appViewHost?.setActive(body.id, body.active === true);
  });

  onTrusted("litecode:app-view-destroy", "workbench", (_event, id: unknown) => {
    const key = typeof id === "string" ? id : "";
    appViewHost?.destroy(key);
    appViewPartitions.delete(key);
    appViewOrigins.delete(key);
  });
}

function registerIpc(): void {
  registerBrowserIpc();
  registerAppViewIpc();
  registerNativeViewIpc();

  onTrusted("litecode:get-auth-token", "workbench", (event) => {
    event.returnValue = authToken || undefined;
  });

  onTrusted("litecode:get-session-mode", "workbench", (event) => {
    event.returnValue = sessionMode;
  });

  handleTrusted("litecode:component-status", "workbench", (_e, id: unknown) => {
    if (!isComponentId(id)) throw new Error("Unknown component");
    return componentManager().status(id);
  });

  // The workbench may only ask for the embed model (code_search); the Linux
  // server bundle is fetched by Open Remote in this process.
  handleTrusted("litecode:ensure-component", "workbench", async (_e, id: unknown) => {
    if (id !== "embed-model") throw new Error("Only embed-model can be requested here");
    if (sessionMode === "remote") {
      throw new Error("Remote workspaces use the embed model on the remote host.");
    }
    const result = await componentManager().ensure(id);
    return { id: result.id, path: result.path, downloaded: result.downloaded };
  });

  onTrusted("litecode:get-ui-theme", "both", (event) => {
    event.returnValue = readUiTheme();
  });

  handleTrusted("litecode:set-ui-theme", "both", (_e, theme: string) => {
    const next: UiThemeName = theme === "light" ? "light" : "default";
    writeUiTheme(next);
  });

  // Dockview snapshots: browser storage in a local workbench is keyed by the
  // sidecar's ephemeral port, so only the host can keep them across launches.
  // Keyed by `currentWorkspace`; null (hub, remote) means no host storage.
  onTrusted("litecode:load-layout", "workbench", (event) => {
    event.returnValue =
      readWorkspaceLayout(app.getPath("userData"), currentWorkspace) ?? undefined;
  });

  handleTrusted("litecode:save-layout", "workbench", (_e, payload: unknown) => {
    writeWorkspaceLayout(app.getPath("userData"), currentWorkspace, payload);
  });

  handleTrusted("litecode:pick-folder", "hub", async (event) => {
    const parent = targetWindow(event);
    const opts = { properties: ["openDirectory" as const] };
    const result = parent
      ? await dialog.showOpenDialog(parent, opts)
      : await dialog.showOpenDialog(opts);
    if (result.canceled || result.filePaths.length === 0) return null;
    return result.filePaths[0] ?? null;
  });

  handleTrusted("litecode:list-recents", "hub", (): RecentWorkspace[] => {
    try {
      return recents?.list() ?? [];
    } catch (error) {
      console.error("Unable to read recent workspaces", error);
      return [];
    }
  });

  handleTrusted("litecode:set-recent-pinned", "hub", (_e, workspacePath: string, pinned: boolean) => {
    return recents?.setPinned(String(workspacePath), Boolean(pinned)) ?? [];
  });

  handleTrusted("litecode:remove-recent", "hub", (_e, workspacePath: string) => {
    return recents?.remove(String(workspacePath)) ?? [];
  });

  handleTrusted("litecode:list-ssh-targets", "hub", (): SavedSshTarget[] => {
    return sshTargets?.list() ?? [];
  });

  handleTrusted("litecode:list-remote-history", "hub", (): SavedSshTarget[] => {
    return sshTargets?.listConnected() ?? [];
  });

  handleTrusted("litecode:start-remote-session", "hub", async (_e, input: StartRemoteInput) => {
    for (const stale of clearAllPendingRemotes()) {
      removeMaterializedKey(stale.keyDirectory);
    }
    return startRemoteSession(input ?? { userAtHost: "" });
  });

  handleTrusted(
    "litecode:list-pending-remote-dirs",
    "hub",
    async (_e, opts: { sessionId: string; path?: string }) => {
      return listPendingRemoteDirs(String(opts?.sessionId ?? ""), opts?.path);
    },
  );

  handleTrusted(
    "litecode:complete-remote-session",
    "hub",
    async (_e, opts: { sessionId: string; workspace: string }) => {
      return completeRemoteSession(String(opts?.sessionId ?? ""), String(opts?.workspace ?? ""));
    },
  );

  handleTrusted("litecode:enter-remote-workbench", "hub", async (_e, sessionId: string) => {
    return enterRemoteWorkbench(String(sessionId ?? ""));
  });

  handleTrusted("litecode:cancel-remote-session", "hub", async (_e, sessionId: string) => {
    await cancelRemoteSession(String(sessionId ?? ""));
  });

  handleTrusted("litecode:reconnect-remote", "hub", async (_e, targetId: string) => {
    await stopManagedRemote();
    return reconnectRemote(String(targetId ?? ""));
  });

  handleTrusted("litecode:set-remote-history-pinned", "hub", (_e, id: string, pinned: boolean) => {
    sshTargets?.setPinned(String(id), Boolean(pinned));
  });

  handleTrusted("litecode:save-ssh-target", "hub", async (_e, target: Omit<SavedSshTarget, "id" | "lastConnectedAt"> & { id?: string; password?: string }) => {
    const store = sshTargets;
    if (!store) throw new Error("SSH target storage is unavailable.");
    const saved = store.save(target);
    if (!target.identityFile && !target.password) return saved;
    if (target.identityFile && target.password) throw new Error("Choose either an SSH private key or password, not both.");
    const credentialKind = target.password ? "password" as const : "private_key" as const;
    const credential = target.password ?? readPrivateKeyFile(target.identityFile!);
    const credentialId = `ssh-${credentialKind}:${saved.id}`;
    await setSecret(credentialId, credential);
    return store.save({ ...saved, credentialId, credentialKind });
  });

  handleTrusted("litecode:remove-ssh-target", "hub", async (_e, id: string) => {
    const removed = sshTargets?.remove(String(id));
    if (removed?.credentialId) await deleteSecret(removed.credentialId);
  });

  handleTrusted("litecode:list-remote-targets", "hub", (): SavedRemoteTarget[] => {
    return remoteTargets?.list() ?? [];
  });

  handleTrusted("litecode:connect-saved-remote", "hub", async (_e, id: string) => {
    await stopManagedRemote();
    return connectSavedRemote(String(id));
  });

  handleTrusted("litecode:set-remote-target-pinned", "hub", (_e, id: string, pinned: boolean) => {
    remoteTargets?.setPinned(String(id), Boolean(pinned));
  });

  handleTrusted("litecode:remove-remote-target", "hub", async (_e, id: string) => {
    const removed = remoteTargets?.remove(String(id));
    if (removed) await deleteSecret(removed.credentialId);
  });

  handleTrusted("litecode:list-ssh-directory", "hub", async (_e, opts: { targetId: string; path?: string }) => {
    const target = sshTargets?.get(String(opts?.targetId ?? ""));
    if (!target) throw new Error(t("desktop.error.sshTargetNotFound"));
    const knownHostsFile = await ensureSshHostTrusted(target);
    const materialized = await materializeSshTarget(target);
    return withMaterializedKeyDirectory(materialized.keyDirectory, async () => {
      const session = new SshSession({ target: materialized.target, askPassCommand: materialized.askPassCommand, knownHostsFile });
      const path = String(opts?.path ?? ".").trim() || ".";
      const entries = await session.listDirectory(path);
      return { path, entries };
    });
  });

  handleTrusted("litecode:connect-ssh-workspace", "hub", async (_e, opts: { targetId: string; workspace: string }) => {
    const targetId = String(opts?.targetId ?? "").trim();
    const workspace = String(opts?.workspace ?? "").trim();
    if (!targetId || !workspace) throw new Error("SSH target and remote workspace are required.");
    return connectManagedSsh(targetId, workspace);
  });

  handleTrusted("litecode:notify-workspace", "workbench", async (_e, workspacePath: string | null) => {
    currentWorkspace = workspacePath ? normalizeWorkspace(String(workspacePath)) : null;
    registry?.setWorkspace(currentWorkspace);
  });

  // Turn-end toast. The renderer already gated visibility and subagent events;
  // this only shows the toast and brings the workbench forward on click.
  onTrusted("litecode:notify-turn-end", "workbench", (_e, payload: unknown) => {
    // Re-check focus at show time: the user may have returned between send and show.
    if (!Notification.isSupported() || mainWindow?.isFocused()) return;
    const body = asRecord(payload);
    const sessionId = typeof body.sessionId === "string" ? body.sessionId : "";
    const title = typeof body.title === "string" ? body.title.trim() : "";
    const text = typeof body.body === "string" ? body.body.trim() : "";
    if (!title && !text) return;
    const toast = new Notification({ title: title || "Litecode", body: text, silent: true });
    liveTurnToasts.push(toast);
    if (liveTurnToasts.length > MAX_LIVE_TOASTS) liveTurnToasts.shift();
    toast.on("click", () => {
      const win = mainWindow;
      if (!win || win.isDestroyed()) return;
      if (win.isMinimized()) win.restore();
      win.show();
      win.moveTop();
      win.focus();
      if (sessionId) {
        win.webContents.send(TURN_NOTIFICATION_CLICK_CHANNEL, { sessionId });
      }
    });
    toast.show();
  });

  /** Real process restart: kill sidecar, spawn with --workspace, reload window. */
  handleTrusted("litecode:open-workspace", "hub", async (_e, workspacePath: string) => {
    if (sessionMode === "remote") {
      throw new Error(t("desktop.error.remoteModeOneWorkspace"));
    }
    const target = normalizeWorkspace(String(workspacePath));
    if (await InstanceRegistry.tryFocusWorkspace(target)) {
      recordRecent(target);
      return { ok: true, focused: true, project: target };
    }
    if (currentWorkspace && normalizeWorkspace(currentWorkspace) === target && sidecar) {
      return { ok: true, focused: false, project: target };
    }
    return relaunchSidecar(target);
  });

  handleTrusted(
    "litecode:connect-remote",
    "hub",
    async (_e, opts: { baseUrl: string; token: string }) => {
      const baseUrl = String(opts?.baseUrl ?? "").trim();
      const token = String(opts?.token ?? "").trim();
      if (!baseUrl || !token) {
        throw new Error("Base URL and token are required.");
      }
      try {
        // Validate URL early so we fail before tearing down local sidecar.
        handshakeUrl(baseUrl, token);
      } catch {
        throw new Error(`Invalid base URL: ${baseUrl}`);
      }
      await stopManagedRemote();
      const result = await connectRemote(baseUrl, token);
      const credentialId = `remote-token:${crypto.createHash("sha256").update(baseUrl).digest("hex")}`;
      await setSecret(credentialId, token);
      remoteTargets?.record(baseUrl, credentialId);
      return result;
    },
  );

  /** Leave remote and spawn a local sidecar for the given workspace. */
  handleTrusted("litecode:return-to-local", "workbench", async (_e, workspacePath: string) => {
    const target = normalizeWorkspace(String(workspacePath));
    if (await InstanceRegistry.tryFocusWorkspace(target)) {
      recordRecent(target);
      return { ok: true, focused: true, project: target };
    }
    authToken = createToken();
    return relaunchSidecar(target);
  });

  handleTrusted("litecode:return-to-hub", "workbench", async () => {
    for (const stale of clearAllPendingRemotes()) {
      removeMaterializedKey(stale.keyDirectory);
    }
    await sidecarLifecycle.run(async () => {
      cancelSidecarRestart();
      sidecarRecovery.reset();
      await stopSidecar(sidecar);
      await stopManagedRemote();
      sidecar = null;
      syncDevUpstream(null);
      sessionMode = "local";
      authToken = createToken();
      currentWorkspace = null;
      registry?.setWorkspace(null);
    });
    if (mainWindow && mainWindow.isDestroyed() === false) {
      await loadHub(mainWindow);
    }
  });

  handleTrusted("litecode:window-minimize", "both", (event) => {
    targetWindow(event)?.minimize();
  });

  handleTrusted("litecode:window-maximize-toggle", "both", (event) => {
    const win = targetWindow(event);
    if (!win) return false;
    if (win.isMaximized()) {
      win.unmaximize();
      return false;
    }
    win.maximize();
    return true;
  });

  handleTrusted("litecode:window-is-maximized", "both", (event) => {
    return targetWindow(event)?.isMaximized() ?? false;
  });

  handleTrusted("litecode:window-close", "both", (event) => {
    targetWindow(event)?.close();
  });

  const popoutChild = (dockId: unknown): BrowserWindow | null => {
    if (typeof dockId !== "string" || !isPopoutDockId(dockId)) return null;
    const child = popoutWindows.get(dockId);
    return child && !child.isDestroyed() ? child : null;
  };

  handleTrusted(
    "litecode:popout-set-always-on-top",
    "workbench",
    (_event, dockId: unknown, onTop: unknown) => {
      const child = popoutChild(dockId);
      if (!child) return false;
      child.setAlwaysOnTop(onTop === true);
      return child.isAlwaysOnTop();
    },
  );

  handleTrusted(
    "litecode:popout-window-minimize",
    "workbench",
    (_event, dockId: unknown) => {
      popoutChild(dockId)?.minimize();
    },
  );

  handleTrusted(
    "litecode:popout-window-maximize-toggle",
    "workbench",
    (_event, dockId: unknown) => {
      const child = popoutChild(dockId);
      if (!child) return false;
      if (child.isMaximized()) {
        child.unmaximize();
        return false;
      }
      child.maximize();
      return true;
    },
  );

  handleTrusted(
    "litecode:popout-window-is-maximized",
    "workbench",
    (_event, dockId: unknown) => {
      return popoutChild(dockId)?.isMaximized() ?? false;
    },
  );
}

app.whenReady().then(async () => {
  // Windows toast attribution: the installer's Start Menu shortcut carries this id.
  // Only the installed exe may own it: in dev Electron writes its notification
  // shortcut for this AUMID pointing at electron.exe, and Windows then paints the
  // generic Electron icon for both the dev shell and the installed app.
  if (process.platform === "win32") {
    app.setAppUserModelId(app.isPackaged ? DESKTOP_APP_ID : DESKTOP_DEV_APP_ID);
  }
  initDesktopI18n(resolveLocale(app.getLocale()));
  registerIpc();
  await startBrowserAutomationBridge();
  recents = new RecentWorkspaces(path.join(app.getPath("userData"), "recent-workspaces.json"));
  remoteTargets = new RemoteTargets(path.join(app.getPath("userData"), "remote-targets.json"));
  sshTargets = new SshTargets(path.join(app.getPath("userData"), "ssh-targets.json"));
  try {
    await boot();
  } catch (err) {
    // WP-169: the user closed the skeleton; the normal quit path cleans up.
    if (isBootAborted(err)) return;
    const message = err instanceof Error ? err.message : String(err);
    // DESK-04: if boot fails after the sidecar was spawned (e.g. health
    // never verifies), stop it so we never leave an orphan server process.
    await stopSidecar(sidecar).catch(() => undefined);
    sidecar = null;
    syncDevUpstream(null);
    dialog.showErrorBox(t("desktop.dialog.bootFailed.title"), message);
    app.exit(1);
  }
});

app.on("window-all-closed", () => {
  // A failed boot shows its error dialog and exits itself.
  if (closingFailedBoot) return;
  if (process.platform !== "darwin") {
    app.quit();
  }
});

app.on("before-quit", (e) => {
  if (quitting) return;
  quitting = true;
  e.preventDefault();
  void (async () => {
    cancelTunnelReconnect();
    cancelSidecarRestart();
    try {
      componentManagerInstance?.abortAll();
    } catch {
      /* ignore */
    }
    registry?.stop();
    await browserBridge?.close().catch(() => undefined);
    browserBridge = null;
    // WP-169: skeleton closed while the sidecar was still starting.
    await bootAbortCleanup?.catch(() => undefined);
    await sidecarLifecycle.run(async () => {
      await stopSidecar(sidecar);
      await stopManagedRemote();
      sidecar = null;
      syncDevUpstream(null);
    });
    app.exit(0);
  })();
});
