import { createHash, randomBytes } from "node:crypto";
import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { stat } from "node:fs/promises";

/**
 * Connection details for system OpenSSH. Authentication is deliberately left
 * to OpenSSH: callers may provide `identityFile`, or use `host` as an SSH
 * config alias. Passwords and private-key material are never persisted here;
 * a caller may provide an ephemeral SSH_ASKPASS helper for a password.
 */
export interface SshTarget {
  /** A hostname, IP address, or safe SSH config alias (never `user@host`). */
  host: string;
  /** Optional remote account; SSH config may supply this when omitted. */
  user?: string;
  port?: number;
  identityFile?: string;
}

export interface SshSessionConfig {
  target: SshTarget;
  /** Override the system command names only when a platform requires it. */
  sshCommand?: string;
  scpCommand?: string;
  connectTimeoutSeconds?: number;
  /** Ephemeral executable used by OpenSSH to obtain a password. */
  askPassCommand?: string;
  /** Private known_hosts file owned by Litecode after explicit user approval. */
  knownHostsFile?: string;
}

export interface RemoteDirectory {
  /** Canonical remote home directory. */
  home: string;
}

export interface RemoteDirectoryEntry {
  name: string;
}

export type InstallTarProgressStage = "upload" | "verify" | "extract" | "ready";

export interface InstallTarProgress {
  stage: InstallTarProgressStage;
  /** 0..1 within the install pipeline. */
  ratio: number;
  message: string;
}

export interface InstallTarOptions {
  localTarPath: string;
  /** Lowercase or uppercase SHA-256 hex digest of the complete tar archive. */
  sha256: string;
  /** Relative directory beneath the remote home, default `.litecode`. */
  destination?: string;
  onProgress?: (progress: InstallTarProgress) => void;
}

export interface RemoteServeOptions {
  /** Remote port. The service is always bound to 127.0.0.1. */
  port: number;
  /** Passed only as LITECODE_TOKEN in the remote process environment. */
  token: string;
  /** Relative workspace directory below the remote home. */
  workspace: string;
  /**
   * Litecode executable path or command name. It is shell-quoted, never
   * interpolated. Defaults to `litecode` from the remote PATH.
   */
  executable?: string;
}

export interface RemoteServeHandle {
  readonly id: string;
  readonly pid: number;
  readonly port: number;
  readonly workspace: string;
}

export interface SshTunnelHandle {
  readonly localPort: number;
  readonly remotePort: number;
  readonly process: ChildProcess;
}

export class SshCommandError extends Error {
  constructor(
    message: string,
    readonly command: string,
    readonly args: readonly string[],
    readonly exitCode: number | null,
    readonly stdout: Buffer,
    readonly stderr: Buffer,
  ) {
    super(message);
    this.name = "SshCommandError";
  }
}

const SAFE_HOST = /^(?:[A-Za-z0-9](?:[A-Za-z0-9._-]{0,251}[A-Za-z0-9])?|[A-Fa-f0-9:]+)$/;
const SAFE_USER = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const SHA256 = /^[a-fA-F0-9]{64}$/;
/** Written beside the extracted server; stores the source tar SHA-256 (not app version). */
const BUNDLE_HASH_FILE = ".bundle.sha256";

/** Reject values which could change SSH's option parsing or destination form. */
export function validateSshTarget(target: SshTarget): SshTarget {
  if (!target || typeof target !== "object") throw new Error("SSH target is required.");
  const host = target.host?.trim();
  if (!host || host !== target.host || !SAFE_HOST.test(host) || host.startsWith("-")) {
    throw new Error("SSH host must be a hostname, IP address, or SSH config alias.");
  }
  if (target.user !== undefined && (!SAFE_USER.test(target.user) || target.user !== target.user.trim())) {
    throw new Error("SSH user contains unsupported characters.");
  }
  if (target.port !== undefined && (!Number.isInteger(target.port) || target.port < 1 || target.port > 65535)) {
    throw new Error("SSH port must be an integer between 1 and 65535.");
  }
  if (target.identityFile !== undefined && (!target.identityFile.trim() || /[\0\r\n]/.test(target.identityFile))) {
    throw new Error("SSH identity file must be a non-empty local path.");
  }
  return { ...target, host };
}

/** Quote one value for a POSIX remote shell. Do not use this for local spawn. */
export function posixShellQuote(value: string): string {
  if (/[\0\r\n]/.test(value)) throw new Error("Remote command values cannot contain NUL or newlines.");
  return `'${value.replace(/'/g, "'\"'\"'")}'`;
}

/**
 * Require a relative POSIX path beneath home. This rejects traversal before
 * any remote command runs; remote operations canonicalize it again to reject
 * symlinks which resolve outside home.
 */
export function relativeHomePath(value: string, field = "remote path"): string {
  if (typeof value !== "string" || !value || /[\0\r\n\\]/.test(value) || value.startsWith("/")) {
    throw new Error(`${field} must be a non-empty relative POSIX path.`);
  }
  const parts = value.split("/");
  if (parts.some((part) => !part || part === "." || part === "..")) {
    throw new Error(`${field} must not contain empty, dot, or traversal segments.`);
  }
  return parts.join("/");
}

function validateConfig(config: SshSessionConfig): Required<Pick<SshSessionConfig, "sshCommand" | "scpCommand" | "connectTimeoutSeconds">> & SshSessionConfig {
  const target = validateSshTarget(config.target);
  const timeout = config.connectTimeoutSeconds ?? 15;
  if (!Number.isInteger(timeout) || timeout < 1 || timeout > 300) {
    throw new Error("SSH connect timeout must be an integer between 1 and 300 seconds.");
  }
  for (const [name, command] of [["ssh", config.sshCommand ?? "ssh"], ["scp", config.scpCommand ?? "scp"]] as const) {
    if (!command || /[\0\r\n]/.test(command)) throw new Error(`${name} command is invalid.`);
  }
  if (config.askPassCommand !== undefined && (!config.askPassCommand || /[\0\r\n]/.test(config.askPassCommand))) {
    throw new Error("SSH askpass command is invalid.");
  }
  if (config.knownHostsFile !== undefined && (!config.knownHostsFile || /[\0\r\n]/.test(config.knownHostsFile))) {
    throw new Error("SSH known_hosts file is invalid.");
  }
  return { ...config, target, sshCommand: config.sshCommand ?? "ssh", scpCommand: config.scpCommand ?? "scp", connectTimeoutSeconds: timeout };
}

/** Construct fixed OpenSSH option arguments. No shell is involved. */
export function buildSshArgs(config: SshSessionConfig, remoteCommand?: string): string[] {
  const safe = validateConfig(config);
  const args = ["-o", safe.askPassCommand ? "BatchMode=no" : "BatchMode=yes", "-o", `ConnectTimeout=${safe.connectTimeoutSeconds}`];
  if (safe.knownHostsFile) args.push("-o", `UserKnownHostsFile=${safe.knownHostsFile}`, "-o", "StrictHostKeyChecking=yes");
  if (safe.target.port) args.push("-p", String(safe.target.port));
  if (safe.target.identityFile) args.push("-i", safe.target.identityFile);
  if (safe.target.user) args.push("-l", safe.target.user);
  args.push(safe.target.host);
  if (remoteCommand !== undefined) args.push(remoteCommand);
  return args;
}

function scpHost(target: SshTarget): string {
  const host = target.host.includes(":") ? `[${target.host}]` : target.host;
  return target.user ? `${target.user}@${host}` : host;
}

/**
 * Remote path for scp destination URIs. Unlike ssh `sh -lc` commands, scp must
 * not wrap the path in shell quotes — quotes become part of the filename.
 */
export function scpRemoteDest(remotePath: string): string {
  if (!remotePath || /[\0\r\n]/.test(remotePath)) {
    throw new Error("SCP remote path cannot be empty or contain NUL/newlines.");
  }
  if (!remotePath.startsWith("/")) {
    throw new Error("SCP remote path must be absolute.");
  }
  if (/['"\\;`|&<>()]/.test(remotePath)) {
    throw new Error("SCP remote path contains unsupported characters.");
  }
  return remotePath.replace(/ /g, "\\ ");
}

/** Construct a safe scp upload invocation. */
export function buildScpUploadArgs(config: SshSessionConfig, localPath: string, remotePath: string): string[] {
  const safe = validateConfig(config);
  if (!localPath || /[\0\r\n]/.test(localPath) || !remotePath || /[\0\r\n]/.test(remotePath)) {
    throw new Error("SCP paths cannot be empty or contain NUL/newlines.");
  }
  const args = ["-o", safe.askPassCommand ? "BatchMode=no" : "BatchMode=yes", "-o", `ConnectTimeout=${safe.connectTimeoutSeconds}`];
  if (safe.knownHostsFile) args.push("-o", `UserKnownHostsFile=${safe.knownHostsFile}`, "-o", "StrictHostKeyChecking=yes");
  if (safe.target.port) args.push("-P", String(safe.target.port));
  if (safe.target.identityFile) args.push("-i", safe.target.identityFile);
  args.push(localPath, `${scpHost(safe.target)}:${scpRemoteDest(remotePath)}`);
  return args;
}

function remoteCommand(script: string): string {
  return `sh -lc ${posixShellQuote(script)}`;
}

function absoluteHomePath(home: string, relative: string): string {
  return `${home}/${relative}`;
}

async function run(command: string, args: readonly string[], askPassCommand?: string): Promise<{ stdout: Buffer; stderr: Buffer }> {
  return new Promise((resolve, reject) => {
    let child;
    try {
      child = spawn(command, args, {
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
        env: askPassCommand
          ? { ...process.env, SSH_ASKPASS: askPassCommand, SSH_ASKPASS_REQUIRE: "force", DISPLAY: "litecode" }
          : process.env,
      });
    } catch (error) {
      reject(error);
      return;
    }
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout?.on("data", (chunk: Buffer) => stdout.push(Buffer.from(chunk)));
    child.stderr?.on("data", (chunk: Buffer) => stderr.push(Buffer.from(chunk)));
    child.once("error", reject);
    child.once("close", (exitCode) => {
      const out = Buffer.concat(stdout);
      const err = Buffer.concat(stderr);
      if (exitCode === 0) resolve({ stdout: out, stderr: err });
      else {
        const detail = err.length ? `: ${err.toString("utf8").trim()}` : "";
        reject(
          new SshCommandError(
            `${command} exited with code ${exitCode}${detail}`,
            command,
            args,
            exitCode,
            out,
            err,
          ),
        );
      }
    });
  });
}

export type SshHostKey = { entry: string; fingerprint: string };

function fingerprintFromKnownHostsEntry(entry: string): string {
  const parts = entry.trim().split(/\s+/);
  if (parts.length < 3 || !/^[A-Za-z0-9+/]+={0,2}$/.test(parts[2]!)) {
    throw new Error("SSH host returned an invalid host key.");
  }
  return `SHA256:${createHash("sha256")
    .update(Buffer.from(parts[2]!, "base64"))
    .digest("base64")
    .replace(/=+$/, "")}`;
}

/**
 * Discover a host key for an explicit trust decision; it is not trusted yet.
 *
 * Uses the system `ssh` client (same path as VS Code Remote-SSH) with
 * `StrictHostKeyChecking=accept-new` into a throwaway known_hosts file.
 * This negotiates KEX with proper fallback — unlike `ssh-keyscan`, which
 * fails on Windows OpenSSH 9.5 against servers that prefer PQ KEX first.
 *
 * Auth is intentionally disabled: host-key exchange completes before login.
 */
export async function scanSshHostKey(target: SshTarget): Promise<SshHostKey> {
  const safe = validateSshTarget(target);
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-ssh-scan-"));
  const knownHosts = path.join(tmpDir, "known_hosts");
  const emptyHosts = path.join(tmpDir, "empty_global_known_hosts");
  fs.writeFileSync(knownHosts, "");
  fs.writeFileSync(emptyHosts, "");
  try {
    const args = [
      "-o",
      "BatchMode=yes",
      "-o",
      "StrictHostKeyChecking=accept-new",
      "-o",
      `UserKnownHostsFile=${knownHosts}`,
      "-o",
      `GlobalKnownHostsFile=${emptyHosts}`,
      "-o",
      "HashKnownHosts=no",
      "-o",
      "UpdateHostKeys=no",
      "-o",
      "ConnectTimeout=10",
      "-o",
      "PreferredAuthentications=none",
      "-o",
      "PubkeyAuthentication=no",
      "-o",
      "PasswordAuthentication=no",
      "-o",
      "KbdInteractiveAuthentication=no",
    ];
    if (safe.port) args.push("-p", String(safe.port));
    args.push(safe.host, "true");

    let scanError: unknown;
    try {
      await run("ssh", args);
    } catch (error) {
      // Expected: auth is disabled. Host key should still be in knownHosts.
      scanError = error;
    }

    const entry = fs
      .readFileSync(knownHosts, "utf8")
      .split(/\r?\n/)
      .find((line) => line.trim() && !line.startsWith("#"));
    if (!entry) {
      if (scanError) throw scanError;
      throw new Error("SSH host did not provide a host key.");
    }
    return { entry: entry.trim(), fingerprint: fingerprintFromKnownHostsEntry(entry) };
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

/**
 * A short-lived controller around system OpenSSH. It holds no credential
 * material; OpenSSH resolves identity files, agents, and SSH config aliases.
 */
export class SshSession {
  private readonly config: ReturnType<typeof validateConfig>;
  private home: string | null = null;
  private activeServe: RemoteServeHandle | null = null;

  constructor(config: SshSessionConfig) {
    this.config = validateConfig(config);
  }

  async probeRemoteHome(): Promise<RemoteDirectory> {
    const result = await this.exec("home=$(realpath -e -- \"$HOME\") && printf '%s\\n' \"$home\"");
    const home = result.stdout.toString("utf8").trim();
    if (!home.startsWith("/") || /[\0\r\n]/.test(home)) throw new Error("Remote HOME is not a valid absolute POSIX path.");
    this.home = home;
    return { home };
  }

  /** List a relative directory only after canonicalization proves it remains in home. */
  async listDirectory(relativePath = "."): Promise<RemoteDirectoryEntry[]> {
    const relative = relativePath === "." ? "." : relativeHomePath(relativePath, "directory path");
    const home = await this.requireHome();
    const candidate = relative === "." ? home : absoluteHomePath(home, relative);
    const script = [
      `home=${posixShellQuote(home)}`,
      `dir=$(realpath -e -- ${posixShellQuote(candidate)})`,
      'case "$dir" in "$home"|"$home"/*) ;; *) echo "directory escapes remote home" >&2; exit 64;; esac',
      'test -d "$dir"',
      "find \"$dir\" -mindepth 1 -maxdepth 1 -type d -printf '%f\\0'",
    ].join("; ");
    const result = await this.exec(script);
    return result.stdout.toString("utf8").split("\0").filter(Boolean).map((name) => ({ name }));
  }

  /**
   * Upload a verified archive then extract it beneath home. The supplied
   * checksum protects the upload/artifact; callers must still trust its tar
   * contents.
   *
   * Skips upload/extract when the destination already has an executable
   * `litecode` and `.bundle.sha256` matching the expected tar digest.
   */
  async installTar(options: InstallTarOptions): Promise<void> {
    if (!SHA256.test(options.sha256)) throw new Error("Tar SHA-256 must be 64 hexadecimal characters.");
    const destination = relativeHomePath(options.destination ?? ".litecode", "install destination");
    const local = await stat(options.localTarPath);
    if (!local.isFile()) throw new Error("Tar source must be a regular local file.");
    const home = await this.requireHome();
    const report = (progress: InstallTarProgress) => options.onProgress?.(progress);
    const expected = options.sha256.toLowerCase();
    const target = absoluteHomePath(home, destination);

    report({ stage: "verify", ratio: 0.05, message: "Checking installed server bundle…" });
    if (await this.bundleMatchesInstalled(target, expected)) {
      report({ stage: "ready", ratio: 1, message: "Using existing remote server bundle." });
      return;
    }

    const nonce = randomBytes(12).toString("hex");
    const remoteArchive = absoluteHomePath(home, `.litecode-upload-${nonce}.tar`);
    report({ stage: "upload", ratio: 0.1, message: "Uploading Litecode server archive…" });
    await run(this.config.scpCommand, buildScpUploadArgs(this.config, options.localTarPath, remoteArchive), this.config.askPassCommand);
    report({ stage: "verify", ratio: 0.45, message: "Verifying archive checksum…" });
    const script = [
      "set -eu",
      `home=${posixShellQuote(home)}`,
      `archive=${posixShellQuote(remoteArchive)}`,
      `expected=${posixShellQuote(expected)}`,
      `target=${posixShellQuote(target)}`,
      `stamp=${posixShellQuote(`${target}/${BUNDLE_HASH_FILE}`)}`,
      'actual=$( (sha256sum -- "$archive" 2>/dev/null || shasum -a 256 -- "$archive") | awk \'{print $1}\' )',
      '[ "$actual" = "$expected" ] || { echo "tar checksum mismatch" >&2; exit 65; }',
      'mkdir -p -- "$target"',
      'target=$(realpath -e -- "$target")',
      'case "$target" in "$home"|"$home"/*) ;; *) echo "install destination escapes remote home" >&2; exit 64;; esac',
      'printf "extract\\n"',
      'tar -xf "$archive" -C "$target"',
      'test -x "$target/litecode" || chmod +x "$target/litecode"',
      'rm -f -- "$archive"',
      'printf "%s\\n" "$expected" > "$stamp"',
    ].join("; ");
    try {
      report({ stage: "extract", ratio: 0.7, message: "Extracting server under remote home…" });
      await this.exec(script);
      report({ stage: "ready", ratio: 1, message: "Remote server binary is ready." });
    } catch (error) {
      // Best effort cleanup; preserve the primary installation error.
      await this.exec(`rm -f -- ${posixShellQuote(remoteArchive)}`).catch(() => undefined);
      throw error;
    }
  }

  /** True when an extracted bundle matches the local tar SHA-256 (not app version). */
  private async bundleMatchesInstalled(target: string, expectedSha256: string): Promise<boolean> {
    const stamp = `${target}/${BUNDLE_HASH_FILE}`;
    const binary = `${target}/litecode`;
    const script = [
      "set -eu",
      `target=${posixShellQuote(target)}`,
      `stamp=${posixShellQuote(stamp)}`,
      `bin=${posixShellQuote(binary)}`,
      `expected=${posixShellQuote(expectedSha256)}`,
      'test -x "$bin"',
      'test -f "$stamp"',
      'actual=$(tr -d "[:space:]" < "$stamp")',
      '[ "$actual" = "$expected" ]',
    ].join("; ");
    try {
      await this.exec(script);
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Launch Litecode detached on the remote loopback interface. A caller must
   * arrange any SSH port forwarding separately; the server is intentionally
   * not exposed on a network interface.
   */
  async startServe(options: RemoteServeOptions): Promise<RemoteServeHandle> {
    if (!Number.isInteger(options.port) || options.port < 1 || options.port > 65535) {
      throw new Error("Serve port must be an integer between 1 and 65535.");
    }
    if (!options.token || /[\0\r\n]/.test(options.token)) throw new Error("Serve token must be non-empty and single-line.");
    const workspace = relativeHomePath(options.workspace, "workspace");
    const executable = options.executable ?? "litecode";
    if (!executable || /[\0\r\n]/.test(executable)) throw new Error("Litecode executable is invalid.");
    const home = await this.requireHome();
    const id = randomBytes(16).toString("hex");
    const stateDir = absoluteHomePath(home, ".litecode/ssh-sessions");
    const pidFile = `${stateDir}/${id}.pid`;
    const logFile = `${stateDir}/${id}.log`;
    const remoteWorkspace = absoluteHomePath(home, workspace);
    const script = [
      "set -eu",
      `state=${posixShellQuote(stateDir)}`,
      `pidfile=${posixShellQuote(pidFile)}`,
      `log=${posixShellQuote(logFile)}`,
      `bin=${posixShellQuote(executable)}`,
      `workspace=${posixShellQuote(remoteWorkspace)}`,
      `token=${posixShellQuote(options.token)}`,
      'test -d "$workspace"',
      'workspace=$(realpath -e -- "$workspace")',
      `home=${posixShellQuote(home)}`,
      'case "$workspace" in "$home"|"$home"/*) ;; *) echo "workspace escapes remote home" >&2; exit 64;; esac',
      'test -w "$workspace" || { echo "workspace is not writable" >&2; exit 66; }',
      'mkdir -p -- "$state" "$home/.local/share/litecode" "$home/.litecode/snapshots"',
      'umask 077',
      'bindir=$(dirname -- "$bin")',
      'test -x "$bin" || chmod +x "$bin"',
      'cd "$bindir"',
      'HOME="$home" LITECODE_TOKEN="$token" nohup "$bin" --workspace "$workspace" serve --bind "127.0.0.1:' +
        options.port +
        '" --loopback-only --require-auth </dev/null >>"$log" 2>&1 & pid=$!',
      'printf "%s\\n" "$pid" > "$pidfile"',
      'printf "%s\\n" "$pid"',
    ].join("; ");
    const result = await this.exec(script);
    const pid = Number(result.stdout.toString("utf8").trim());
    if (!Number.isSafeInteger(pid) || pid < 1) throw new Error("Remote serve did not return a valid PID.");
    const handle: RemoteServeHandle = { id, pid, port: options.port, workspace };
    this.activeServe = handle;
    return handle;
  }

  /** Last lines from a managed remote serve log (for health-check failures). */
  async readSessionLog(sessionId: string): Promise<string> {
    if (!/^[a-f0-9]{32}$/.test(sessionId)) {
      throw new Error("Invalid remote session id.");
    }
    const home = await this.requireHome();
    const logFile = absoluteHomePath(home, `.litecode/ssh-sessions/${sessionId}.log`);
    const script = [
      `log=${posixShellQuote(logFile)}`,
      'test -f "$log" || exit 0',
      'tail -n 40 "$log"',
    ].join("; ");
    const result = await this.exec(script);
    return result.stdout.toString("utf8").trim();
  }

  /** Stop a server started by this controller, using its private pid file. */
  async stopServe(handle: RemoteServeHandle | null = this.activeServe): Promise<void> {
    if (!handle) return;
    if (!/^[a-f0-9]{32}$/.test(handle.id) || !Number.isSafeInteger(handle.pid) || handle.pid < 1) {
      throw new Error("Invalid remote serve handle.");
    }
    const home = await this.requireHome();
    const pidFile = absoluteHomePath(home, `.litecode/ssh-sessions/${handle.id}.pid`);
    const script = [
      `pidfile=${posixShellQuote(pidFile)}`,
      'test -f "$pidfile" || exit 0',
      'pid=$(cat "$pidfile")',
      'case "$pid" in *[!0-9]*|"") exit 64;; esac',
      'kill "$pid" 2>/dev/null || true',
      'rm -f -- "$pidfile"',
    ].join("; ");
    await this.exec(script);
    if (this.activeServe?.id === handle.id) this.activeServe = null;
  }

  /**
   * Keep the control plane private: the remote serve remains on loopback and
   * Electron reaches it through a local loopback SSH forward.
   */
  startTunnel(localPort: number, remotePort: number): SshTunnelHandle {
    for (const [label, port] of [["local", localPort], ["remote", remotePort]] as const) {
      if (!Number.isInteger(port) || port < 1 || port > 65535) {
        throw new Error(`${label} tunnel port must be an integer between 1 and 65535.`);
      }
    }
    const args = buildSshArgs(this.config);
    const host = args.pop();
    if (!host) throw new Error("SSH target is unavailable.");
    const child = spawn(
      this.config.sshCommand,
      [...args, "-N", "-L", `127.0.0.1:${localPort}:127.0.0.1:${remotePort}`, host],
      {
        stdio: ["ignore", "ignore", "pipe"],
        windowsHide: true,
        env: this.config.askPassCommand
          ? { ...process.env, SSH_ASKPASS: this.config.askPassCommand, SSH_ASKPASS_REQUIRE: "force", DISPLAY: "litecode" }
          : process.env,
      },
    );
    return { localPort, remotePort, process: child };
  }

  stopTunnel(handle: SshTunnelHandle | null): void {
    if (!handle || handle.process.killed) return;
    handle.process.kill();
  }

  private async requireHome(): Promise<string> {
    return this.home ?? (await this.probeRemoteHome()).home;
  }

  private exec(script: string): Promise<{ stdout: Buffer; stderr: Buffer }> {
    return run(this.config.sshCommand, buildSshArgs(this.config, remoteCommand(script)), this.config.askPassCommand);
  }
}
