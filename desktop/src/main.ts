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
  shell,
} from "electron";

import { InstanceRegistry, normalizeWorkspace } from "./instance-registry";
import { handshakeUrl } from "./handshake-url";
import { writeHubPage } from "./hub";
import {
  assertIpcSurface,
  exactHttpOrigin,
  isAllowedNavigation,
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
  REMOTE_PROGRESS_CHANNEL,
  type RemoteProgressEvent,
} from "./remote-progress";
import { startSidecar, stopSidecar, type SidecarHandle } from "./sidecar";
import {
  materializePasswordAskPass,
  materializePrivateKeyFile,
  readPrivateKeyFile,
  removeMaterializedKey,
  withMaterializedKeyDirectory,
} from "./ssh-credential-material";
import { scanSshHostKey, SshSession, type RemoteServeHandle, type SshTunnelHandle } from "./ssh-session";
import { SshTargets, type SavedSshTarget } from "./ssh-targets";
import { readUiTheme, writeUiTheme, type UiThemeName } from "./ui-theme";
import { isAllowedExternalUrl, isAllowedLoadUrl } from "./url-policy";
import { senderOwnedWindow } from "./window-guard";

type SessionMode = "local" | "remote";

let mainWindow: BrowserWindow | null = null;
let sidecar: SidecarHandle | null = null;
let authToken = "";
let sessionMode: SessionMode = "local";
let registry: InstanceRegistry | null = null;
let recents: RecentWorkspaces | null = null;
let remoteTargets: RemoteTargets | null = null;
let sshTargets: SshTargets | null = null;
let managedRemote: {
  session: SshSession;
  serve: RemoteServeHandle;
  tunnel: SshTunnelHandle;
  keyFile?: string;
  keyDirectory?: string;
} | null = null;
let quitting = false;
let currentWorkspace: string | null = null;
let trustContext: IpcTrustContext | null = null;

function createToken(): string {
  return crypto.randomBytes(32).toString("base64url");
}

type BootContent = { kind: "url"; url: string } | { kind: "hub" };

function hubPagePath(): string {
  return path.join(app.getPath("userData"), "hub-index.html");
}

async function loadHub(win: BrowserWindow): Promise<void> {
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
  trustContext = {
    activeSurface: "workbench",
    hubFileUrl: pathToFileURL(hubPagePath()).toString(),
    workbenchOrigin,
  };
  await win.loadURL(url);
}

async function createWindow(content: BootContent): Promise<BrowserWindow> {
  const win = new BrowserWindow({
    width: 1280,
    height: 800,
    show: false,
    frame: false,
    backgroundColor: "#0a0a0a",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
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
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (isAllowedExternalUrl(url)) void shell.openExternal(url);
    return { action: "deny" };
  });
  const guardNavigation = (event: Electron.Event, url: string) => {
    const context = trustContext;
    if (!context || !isAllowedNavigation(context.activeSurface, url, context)) {
      event.preventDefault();
    }
  };
  win.webContents.on("will-navigate", guardNavigation);
  win.webContents.on("will-redirect", guardNavigation);

  if (content.kind === "hub") {
    await loadHub(win);
  } else {
    await safeLoadURL(win, content.url);
  }
  return win;
}

/** Resolve a CLI/env workspace. Absence deliberately starts the local hub. */
function resolveBootWorkspace(): string | null {
  const fromEnv =
    process.env.LITECODE_WORKSPACE?.trim() ||
    process.argv.find((a, i, arr) => arr[i - 1] === "--workspace") ||
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

function linuxBundlePaths(): { tar: string; checksum: string } {
  const fileName = "litecode-server-linux-x64.tar.gz";
  const packaged = path.join(process.resourcesPath, "linux", fileName);
  const development = path.resolve(app.getAppPath(), "..", "dist", "linux", fileName);
  const tar = fs.existsSync(packaged) ? packaged : development;
  const checksum = `${tar}.sha256`;
  if (!fs.existsSync(tar) || !fs.existsSync(checksum)) {
    throw new Error(
      "Linux server bundle is missing under dist/linux/. Run ./scripts/package_linux.sh in WSL, then retry.",
    );
  }
  return { tar, checksum };
}

async function materializeSshTarget(target: SavedSshTarget): Promise<{
  target: SavedSshTarget;
  keyFile?: string;
  keyDirectory?: string;
  askPassCommand?: string;
}> {
  if (!target.credentialId) return { target };
  const credential = await getSecret(target.credentialId);
  if (!credential) throw new Error("The saved SSH credential is unavailable.");
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
      buttons: ["Trust and continue", "Cancel"],
      defaultId: 1,
      cancelId: 1,
      title: "Verify SSH host identity",
      message: `First connection to ${key}`,
      detail: `Verify this fingerprint with the host administrator before continuing:\n${discovered.fingerprint}`,
    };
    const result = mainWindow
      ? await dialog.showMessageBox(mainWindow, options)
      : await dialog.showMessageBox(options);
    if (result.response !== 0) throw new Error("SSH host identity was not approved.");
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

async function stopManagedRemote(): Promise<void> {
  const active = managedRemote;
  managedRemote = null;
  if (!active) return;
  active.session.stopTunnel(active.tunnel);
  await active.session.stopServe(active.serve).catch((error) => {
    console.warn("Failed to stop managed remote workspace", error);
  });
  removeMaterializedKey(active.keyDirectory);
}

async function installLinuxBundle(session: SshSession): Promise<void> {
  const { tar, checksum } = linuxBundlePaths();
  const hash = fs.readFileSync(checksum, "utf8").trim().split(/\s+/)[0];
  if (!hash) throw new Error("Linux server checksum file is invalid.");
  await session.installTar({
    localTarPath: tar,
    sha256: hash,
    destination: remoteBundleInstallDir(),
    onProgress: (progress) => {
      emitRemoteProgress({
        stage: progress.stage,
        ratio: 0.15 + progress.ratio * 0.7,
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
  emitRemoteProgress({ stage: "starting", ratio: 0.9, message: "Starting remote Litecode serve…" });
  const remotePort = await unusedLoopbackPort();
  const localPort = await unusedLoopbackPort();
  const token = createToken();
  const serve = await session.startServe({
    port: remotePort,
    token,
    workspace,
    executable: `${home}/${remoteBundleInstallDir()}/litecode`,
  });
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

async function connectManagedSsh(targetId: string, workspace: string): Promise<{ ok: true; mode: "remote" }> {
  const target = sshTargets?.get(targetId);
  if (!target) throw new Error("SSH target was not found.");
  emitRemoteProgress({ stage: "authenticating", ratio: 0.05, message: "Connecting over SSH…" });
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
    managedRemote = {
      session,
      serve,
      tunnel,
      keyFile: materialized.keyFile,
      keyDirectory: materialized.keyDirectory,
    };
    sshTargets?.updateConnection(target.id, workspace);
    emitRemoteProgress({ stage: "attaching", ratio: 1, message: "Opening workspace…" });
    return connectRemote(`http://127.0.0.1:${started.localPort}`, started.token);
  } catch (error) {
    session.stopTunnel(tunnel);
    await session.stopServe(serve).catch(() => undefined);
    removeMaterializedKey(materialized.keyDirectory);
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
  const parsed = parseUserAtHost(input.userAtHost);
  const authMode = input.authMode ?? (input.password ? "password" : input.identityFile ? "private_key" : "agent");
  if (authMode === "password" && !input.password?.trim()) {
    throw new Error("Password is required.");
  }
  if (authMode === "private_key" && !input.identityFile?.trim()) {
    throw new Error("Private key path is required.");
  }

  emitRemoteProgress({ stage: "authenticating", ratio: 0.05, message: "Verifying SSH host and credentials…" });

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
    emitRemoteProgress({ stage: "ready", ratio: 0.88, message: "Choose a workspace folder." });
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
  const pending = getPendingRemote(sessionId);
  const relative = String(workspace ?? "").trim() || ".";
  if (relative === ".") {
    throw new Error("Choose a workspace folder below the remote home (not home itself).");
  }

  const started = await startRemoteServeAndTunnel(pending.session, pending.home, relative);
  await stopManagedRemote();
  managedRemote = {
    session: pending.session,
    serve: started.serve,
    tunnel: started.tunnel,
    keyDirectory: pending.keyDirectory,
  };

  const baseUrl = `http://127.0.0.1:${started.localPort}`;
  pending.ready = { token: started.token, baseUrl, workspace: relative };
  setPendingRemote(pending);

  emitRemoteProgress({
    stage: "ready",
    ratio: 0.96,
    message: "Remote session ready. Review the token, then enter the workspace.",
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
    throw new Error("Remote session is not ready. Choose a workspace first.");
  }
  emitRemoteProgress({ stage: "attaching", ratio: 1, message: "Opening workspace…" });
  const result = await connectRemote(pending.ready.baseUrl, pending.ready.token);
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
    throw new Error("This remote has never completed a connection.");
  }
  return connectManagedSsh(target.id, target.lastWorkspace);
}

/**
 * Stop current sidecar (if any) and start a new one rooted at `workspace`.
 * Reloads the main window onto the new READY URL (new ephemeral port).
 */
async function relaunchSidecar(workspacePath: string): Promise<{ ok: true; project: string }> {
  const workspace = requireWorkspaceDirectory(workspacePath);
  await stopSidecar(sidecar);
  sidecar = null;

  sessionMode = "local";
  sidecar = await startSidecar({
    token: authToken,
    parentPid: process.pid,
    workspace,
  });
  await verifySidecarAttached(sidecar);
  currentWorkspace = workspace;
  registry?.setWorkspace(workspace);
  recordRecent(workspace);

  if (mainWindow && !mainWindow.isDestroyed()) {
    await safeLoadURL(mainWindow, sidecar.readyUrl);
  }
  return { ok: true, project: workspace };
}

async function connectRemote(baseUrl: string, token: string): Promise<{ ok: true; mode: "remote" }> {
  const url = handshakeUrl(baseUrl, token);
  await waitForWorkspaceHealth(healthUrlFromBase(baseUrl));
  await stopSidecar(sidecar);
  sidecar = null;
  sessionMode = "remote";
  authToken = token;
  currentWorkspace = null;
  registry?.setWorkspace(null);

  if (mainWindow && !mainWindow.isDestroyed()) {
    await safeLoadURL(mainWindow, url);
  }
  return { ok: true, mode: "remote" };
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
  const remoteUrl =
    process.env.LITECODE_REMOTE_URL?.trim() ||
    process.env.LITECODE_DEV_URL?.trim() ||
    "";
  let content: BootContent;

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
    await waitForWorkspaceHealth(
      healthUrlFromBase(remoteUrl),
      expected ? normalizeWorkspace(expected) : undefined,
    );
    content = { kind: "url", url: handshakeUrl(remoteUrl, authToken) };
    if (expected) {
      currentWorkspace = normalizeWorkspace(expected);
    }
  } else {
    sessionMode = "local";
    authToken = createToken();
    const workspace = resolveBootWorkspace();
    if (workspace) {
      sidecar = await startSidecar({
        token: authToken,
        parentPid: process.pid,
        workspace,
      });
      await verifySidecarAttached(sidecar);
      currentWorkspace = workspace;
      recordRecent(workspace);
      content = { kind: "url", url: sidecar.readyUrl };
    } else {
      content = { kind: "hub" };
    }
  }

  mainWindow = await createWindow(content);
  registry = new InstanceRegistry();
  await registry.start(mainWindow);
  if (currentWorkspace) {
    registry.setWorkspace(currentWorkspace);
  }

  mainWindow.on("closed", () => {
    mainWindow = null;
  });
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

function registerIpc(): void {
  onTrusted("litecode:get-auth-token", "workbench", (event) => {
    event.returnValue = authToken || undefined;
  });

  onTrusted("litecode:get-session-mode", "workbench", (event) => {
    event.returnValue = sessionMode;
  });

  onTrusted("litecode:get-ui-theme", "both", (event) => {
    event.returnValue = readUiTheme();
  });

  handleTrusted("litecode:set-ui-theme", "both", (_e, theme: string) => {
    const next: UiThemeName = theme === "light" ? "light" : "default";
    writeUiTheme(next);
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
    if (!target) throw new Error("SSH target was not found.");
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

  handleTrusted("litecode:focus-workspace", "workbench", async (_e, workspacePath: string) => {
    return InstanceRegistry.tryFocusWorkspace(String(workspacePath));
  });

  handleTrusted("litecode:notify-workspace", "workbench", async (_e, workspacePath: string | null) => {
    currentWorkspace = workspacePath ? normalizeWorkspace(String(workspacePath)) : null;
    registry?.setWorkspace(currentWorkspace);
  });

  /** Real process restart: kill sidecar, spawn with --workspace, reload window. */
  handleTrusted("litecode:open-workspace", "hub", async (_e, workspacePath: string) => {
    if (sessionMode === "remote") {
      throw new Error(
        "Remote mode: one process = one workspace on the server. Use Options → Home to open a local folder, or reconnect from Remote history.",
      );
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
    await stopSidecar(sidecar);
    await stopManagedRemote();
    sidecar = null;
    sessionMode = "local";
    authToken = createToken();
    currentWorkspace = null;
    registry?.setWorkspace(null);
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
}

app.whenReady().then(async () => {
  registerIpc();
  recents = new RecentWorkspaces(path.join(app.getPath("userData"), "recent-workspaces.json"));
  remoteTargets = new RemoteTargets(path.join(app.getPath("userData"), "remote-targets.json"));
  sshTargets = new SshTargets(path.join(app.getPath("userData"), "ssh-targets.json"));
  try {
    await boot();
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    // DESK-04: if boot fails after the sidecar was spawned (e.g. health
    // never verifies), stop it so we never leave an orphan server process.
    await stopSidecar(sidecar).catch(() => undefined);
    sidecar = null;
    dialog.showErrorBox("Litecode failed to start", message);
    app.exit(1);
  }
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});

app.on("before-quit", (e) => {
  if (quitting) return;
  quitting = true;
  e.preventDefault();
  void (async () => {
    registry?.stop();
    await stopSidecar(sidecar);
    await stopManagedRemote();
    sidecar = null;
    app.exit(0);
  })();
});
