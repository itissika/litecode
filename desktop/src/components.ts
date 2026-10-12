/**
 * On-demand runtime components (slim Windows installer).
 *
 * The NSIS installer no longer embeds the granite embed model or the Linux
 * remote-server tar. The first time code_search (embed) or Open Remote needs
 * one, Electron main downloads it from this repo's GitHub Release for the
 * running app version, verifies size + sha256 against the release's
 * `litecode-components.json`, and installs it under the existing
 * `%LOCALAPPDATA%\litecode\bundles` convention. The Rust sidecar already
 * searches that directory, so a component installed here is picked up
 * without restarting anything.
 *
 * Overrides (offline / dev / mirrors):
 *   LITECODE_COMPONENTS_BASE=<dir | file:// URL>  manifest + files from a local folder
 *   LITECODE_COMPONENTS_BASE=<https URL>          manifest + files from that base URL
 */
import { createHash, randomBytes } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream";
import zlib from "node:zlib";

import { BUNDLED_MODEL_REL, LINUX_TAR_NAME, isModelDirReady, litecodeBundlesDir } from "./bundle-paths";

export const COMPONENTS_MANIFEST_NAME = "litecode-components.json";
export const COMPONENTS_RELEASE_REPO = "itissika/litecode";
export const COMPONENTS_BASE_ENV = "LITECODE_COMPONENTS_BASE";
export const COMPONENT_PROGRESS_CHANNEL = "litecode:component-progress";
const INSTALL_RECORD = "components.json";
const IDLE_TIMEOUT_MS = 45_000;
const MANIFEST_TIMEOUT_MS = 20_000;
const NETWORK_ATTEMPTS = 3;

export type ComponentId = "embed-model" | "linux-server";
export const COMPONENT_IDS: readonly ComponentId[] = ["embed-model", "linux-server"];

export function isComponentId(value: unknown): value is ComponentId {
  return typeof value === "string" && (COMPONENT_IDS as readonly string[]).includes(value);
}

export type ComponentFormat = "file" | "tar.gz";

export type ManifestComponent = {
  id: string;
  version: string;
  /** `any` | `linux-x64` | `win-x64` — platform the payload runs on. */
  platform: string;
  /** Release asset name. */
  file: string;
  url: string;
  size: number;
  sha256: string;
  format: ComponentFormat;
  /** Inclusive app version range this payload works with. `max` omitted = open. */
  app_version_range: { min: string; max?: string };
};

export type ComponentsManifest = {
  schema: 1;
  app_version: string;
  tag: string;
  repo: string;
  generated_at?: string;
  components: ManifestComponent[];
};

export type ComponentPhase = "resolving" | "downloading" | "verifying" | "installing" | "done" | "error";

/** Rich download lifecycle for pause/stop/retry UI (口径 #7). */
export type ComponentDownloadStatus =
  | "idle"
  | "downloading"
  | "paused"
  | "stopped"
  | "error"
  | "done";

export type ComponentProgressEvent = {
  id: ComponentId;
  phase: ComponentPhase;
  /** Richer status mirror (defaults derived from phase when omitted by older callers). */
  status?: ComponentDownloadStatus;
  /** Bytes received (downloading/verifying). */
  received?: number;
  /** Expected bytes. */
  total?: number;
  /** 0..100 */
  percent?: number;
  /** Sliding-window throughput while downloading. */
  bytesPerSecond?: number;
  message: string;
  error?: string;
};

export type ComponentSource =
  | { kind: "github"; manifestUrl: string; allowedPrefix: string }
  | { kind: "url"; base: string; manifestUrl: string }
  | { kind: "dir"; dir: string };

const COMPONENT_LABEL: Record<ComponentId, string> = {
  "embed-model": "Embedding model",
  "linux-server": "Linux server bundle",
};

/* ------------------------------------------------------------------ */
/* Versions                                                            */
/* ------------------------------------------------------------------ */

function versionParts(v: string): number[] {
  const core = v.trim().replace(/^v/i, "").split(/[-+]/)[0] ?? "";
  return core.split(".").map((p) => {
    const n = Number.parseInt(p, 10);
    return Number.isFinite(n) ? n : 0;
  });
}

export function compareVersions(a: string, b: string): number {
  const pa = versionParts(a);
  const pb = versionParts(b);
  const len = Math.max(pa.length, pb.length, 3);
  for (let i = 0; i < len; i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  return 0;
}

export function versionInRange(version: string, range: { min: string; max?: string }): boolean {
  if (compareVersions(version, range.min) < 0) return false;
  if (range.max && compareVersions(version, range.max) > 0) return false;
  return true;
}

/* ------------------------------------------------------------------ */
/* Source + manifest                                                   */
/* ------------------------------------------------------------------ */

export function releaseDownloadPrefix(repo = COMPONENTS_RELEASE_REPO): string {
  return `https://github.com/${repo}/releases/download/`;
}

export function resolveComponentSource(
  appVersion: string,
  env: NodeJS.ProcessEnv = process.env,
): ComponentSource {
  const override = env[COMPONENTS_BASE_ENV]?.trim();
  if (override) {
    if (/^https?:\/\//i.test(override)) {
      const base = override.replace(/\/+$/, "");
      return { kind: "url", base, manifestUrl: `${base}/${COMPONENTS_MANIFEST_NAME}` };
    }
    const dir = /^file:\/\//i.test(override) ? fileURLToPath(override) : path.resolve(override);
    return { kind: "dir", dir };
  }
  const prefix = releaseDownloadPrefix();
  const tag = `v${appVersion.replace(/^v/i, "")}`;
  return {
    kind: "github",
    manifestUrl: `${prefix}${tag}/${COMPONENTS_MANIFEST_NAME}`,
    allowedPrefix: prefix,
  };
}

function describeSource(source: ComponentSource): string {
  if (source.kind === "dir") return source.dir;
  return source.manifestUrl;
}

const SHA256_RE = /^[0-9a-f]{64}$/;
const SAFE_FILE_RE = /^[A-Za-z0-9][A-Za-z0-9._+-]*$/;

/** Validate a parsed manifest. Throws a readable error on any malformed field. */
export function parseManifest(raw: unknown): ComponentsManifest {
  const fail = (why: string): never => {
    throw new Error(`Invalid components manifest: ${why}`);
  };
  if (!raw || typeof raw !== "object") fail("not an object");
  const m = raw as Record<string, unknown>;
  if (m.schema !== 1) fail(`unsupported schema ${String(m.schema)}`);
  if (!Array.isArray(m.components)) fail("components must be an array");
  const components = (m.components as unknown[]).map((entry, i) => {
    if (!entry || typeof entry !== "object") fail(`components[${i}] is not an object`);
    const c = entry as Record<string, unknown>;
    const str = (key: string): string => {
      const v = c[key];
      if (typeof v !== "string" || !v.trim()) fail(`components[${i}].${key} missing`);
      return (v as string).trim();
    };
    const sha256 = str("sha256").toLowerCase();
    if (!SHA256_RE.test(sha256)) fail(`components[${i}].sha256 is not a hex sha256`);
    const size = c.size;
    if (typeof size !== "number" || !Number.isSafeInteger(size) || size <= 0) {
      fail(`components[${i}].size must be a positive integer`);
    }
    const format = str("format");
    if (format !== "file" && format !== "tar.gz") fail(`components[${i}].format ${format} unsupported`);
    const file = str("file");
    if (!SAFE_FILE_RE.test(file)) fail(`components[${i}].file ${file} is not a plain asset name`);
    const range = c.app_version_range as Record<string, unknown> | undefined;
    if (!range || typeof range.min !== "string" || !range.min.trim()) {
      fail(`components[${i}].app_version_range.min missing`);
    }
    if (range!.max !== undefined && typeof range!.max !== "string") {
      fail(`components[${i}].app_version_range.max must be a string`);
    }
    return {
      id: str("id"),
      version: str("version"),
      platform: str("platform"),
      file,
      url: str("url"),
      size: size as number,
      sha256,
      format: format as ComponentFormat,
      app_version_range: {
        min: (range!.min as string).trim(),
        ...(typeof range!.max === "string" && range!.max.trim() ? { max: range!.max.trim() } : {}),
      },
    } satisfies ManifestComponent;
  });
  return {
    schema: 1,
    app_version: typeof m.app_version === "string" ? m.app_version : "",
    tag: typeof m.tag === "string" ? m.tag : "",
    repo: typeof m.repo === "string" ? m.repo : "",
    ...(typeof m.generated_at === "string" ? { generated_at: m.generated_at } : {}),
    components,
  };
}

export function selectComponent(
  manifest: ComponentsManifest,
  id: ComponentId,
  appVersion: string,
): ManifestComponent {
  const candidates = manifest.components.filter((c) => c.id === id);
  if (candidates.length === 0) {
    throw new Error(`${COMPONENT_LABEL[id]} is not listed in ${COMPONENTS_MANIFEST_NAME}.`);
  }
  const compatible = candidates.filter((c) => versionInRange(appVersion, c.app_version_range));
  if (compatible.length === 0) {
    const ranges = candidates
      .map((c) => `${c.app_version_range.min}..${c.app_version_range.max ?? "*"}`)
      .join(", ");
    throw new Error(
      `No ${COMPONENT_LABEL[id]} compatible with Litecode ${appVersion} (manifest offers ${ranges}).`,
    );
  }
  return compatible.sort((a, b) => compareVersions(b.version, a.version))[0]!;
}

/** Where to fetch one component's payload from, after enforcing the source policy. */
export function componentPayloadLocation(
  source: ComponentSource,
  component: ManifestComponent,
): { kind: "url"; url: string } | { kind: "file"; file: string } {
  switch (source.kind) {
    case "dir":
      return { kind: "file", file: path.join(source.dir, component.file) };
    case "url":
      return { kind: "url", url: `${source.base}/${component.file}` };
    case "github": {
      const url = component.url;
      if (!url.startsWith(source.allowedPrefix)) {
        throw new Error(
          `Refusing ${component.id} download from ${url}: only ${source.allowedPrefix}… (GitHub Releases) is allowed.`,
        );
      }
      return { kind: "url", url };
    }
  }
}

/* ------------------------------------------------------------------ */
/* Install record                                                      */
/* ------------------------------------------------------------------ */

export type InstallRecordEntry = {
  version: string;
  sha256: string;
  app_version: string;
  installed_at: string;
};

export type InstallRecord = { components: Partial<Record<ComponentId, InstallRecordEntry>> };

export function readInstallRecord(bundlesDir: string): InstallRecord {
  try {
    const raw = JSON.parse(fs.readFileSync(path.join(bundlesDir, INSTALL_RECORD), "utf8")) as InstallRecord;
    if (raw && typeof raw === "object" && raw.components && typeof raw.components === "object") return raw;
  } catch {
    /* missing or corrupt → empty */
  }
  return { components: {} };
}

function writeInstallRecord(bundlesDir: string, id: ComponentId, entry: InstallRecordEntry): void {
  const record = readInstallRecord(bundlesDir);
  record.components[id] = entry;
  fs.mkdirSync(bundlesDir, { recursive: true });
  const tmp = path.join(bundlesDir, `${INSTALL_RECORD}.${randomBytes(4).toString("hex")}.tmp`);
  fs.writeFileSync(tmp, `${JSON.stringify(record, null, 2)}\n`, "utf8");
  fs.renameSync(tmp, path.join(bundlesDir, INSTALL_RECORD));
}

/* ------------------------------------------------------------------ */
/* Install layout                                                      */
/* ------------------------------------------------------------------ */

export function installedModelDir(bundlesDir: string): string {
  return path.join(bundlesDir, "models", BUNDLED_MODEL_REL);
}

export function installedLinuxTar(bundlesDir: string): string {
  return path.join(bundlesDir, "linux", LINUX_TAR_NAME);
}

/* ------------------------------------------------------------------ */
/* Transfer                                                            */
/* ------------------------------------------------------------------ */

type FetchLike = typeof fetch;

type TransferProgress = (received: number, total: number, bytesPerSecond: number) => void;

class VerificationError extends Error {}

/** User/control aborts — pause keeps `.part`, stop deletes it. */
export class DownloadControlError extends Error {
  constructor(readonly kind: "pause" | "stop") {
    super(kind === "pause" ? "Download paused" : "Download stopped");
    this.name = "DownloadControlError";
  }
}

export function isDownloadControlError(error: unknown): error is DownloadControlError {
  return error instanceof DownloadControlError;
}

function controlKindFromAbort(signal: AbortSignal): "pause" | "stop" | null {
  const reason = (signal as AbortSignal & { reason?: unknown }).reason;
  if (reason instanceof DownloadControlError) return reason.kind;
  if (reason && typeof reason === "object" && "kind" in reason) {
    const kind = (reason as { kind?: unknown }).kind;
    if (kind === "pause" || kind === "stop") return kind;
  }
  return null;
}

/** Sliding window throughput over the last ~2s of progress samples. */
export class SpeedTracker {
  private samples: Array<{ t: number; bytes: number }> = [];

  push(received: number, now = Date.now()): void {
    this.samples.push({ t: now, bytes: received });
    while (this.samples.length > 1 && now - this.samples[0]!.t > 2_000) {
      this.samples.shift();
    }
  }

  bytesPerSecond(): number {
    if (this.samples.length < 2) return 0;
    const first = this.samples[0]!;
    const last = this.samples[this.samples.length - 1]!;
    const dt = (last.t - first.t) / 1000;
    if (dt <= 0) return 0;
    return Math.max(0, (last.bytes - first.bytes) / dt);
  }
}

export function progressPercent(received: number | undefined, total: number | undefined): number {
  if (!total || total <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round(((received ?? 0) / total) * 100)));
}

/** Stable `.part` path so pause/retry can resume the same blob. */
export function componentPartPath(tmpDir: string, id: ComponentId, sha256: string): string {
  return path.join(tmpDir, `${id}-${sha256.slice(0, 16)}.part`);
}

/**
 * Remove leftover `.part` / staging dirs under `bundles/.tmp`.
 * When `maxAgeMs` is 0 (default), clears everything that looks like our temp
 * artefacts — safe at process start when no other download holds them.
 */
export function cleanupComponentTmp(tmpDir: string, maxAgeMs = 0, now = Date.now()): number {
  if (!fs.existsSync(tmpDir)) return 0;
  let removed = 0;
  for (const name of fs.readdirSync(tmpDir)) {
    const full = path.join(tmpDir, name);
    let st: fs.Stats;
    try {
      st = fs.statSync(full);
    } catch {
      continue;
    }
    if (maxAgeMs > 0 && now - st.mtimeMs < maxAgeMs) continue;
    const looksLikePart = name.endsWith(".part");
    const looksLikeStaging = name.startsWith("embed-model-") && st.isDirectory();
    if (!looksLikePart && !looksLikeStaging) continue;
    try {
      fs.rmSync(full, { recursive: true, force: true });
      removed += 1;
    } catch {
      /* ignore busy files */
    }
  }
  return removed;
}

async function fetchManifest(
  source: ComponentSource,
  fetchImpl: FetchLike,
): Promise<ComponentsManifest> {
  if (source.kind === "dir") {
    const file = path.join(source.dir, COMPONENTS_MANIFEST_NAME);
    let text: string;
    try {
      text = fs.readFileSync(file, "utf8");
    } catch (error) {
      throw new Error(`Cannot read ${file}: ${(error as Error).message}`);
    }
    return parseManifest(JSON.parse(text));
  }
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), MANIFEST_TIMEOUT_MS);
  try {
    const res = await fetchImpl(source.manifestUrl, { signal: ctrl.signal, redirect: "follow" });
    if (res.status === 404) {
      throw new Error(
        `No component manifest published for this version (${source.manifestUrl} → 404).`,
      );
    }
    if (!res.ok) throw new Error(`Manifest request failed: HTTP ${res.status} (${source.manifestUrl})`);
    return parseManifest(await res.json());
  } finally {
    clearTimeout(timer);
  }
}

async function streamToFile(
  chunks: AsyncIterable<Uint8Array>,
  dest: string,
  expected: { size: number; sha256: string },
  onProgress: TransferProgress,
  opts: { append?: boolean; startReceived?: number; startHash?: ReturnType<typeof createHash> } = {},
): Promise<void> {
  const hash = opts.startHash ?? createHash("sha256");
  const startReceived = opts.startReceived ?? 0;
  const handle = await fs.promises.open(dest, opts.append ? "a" : "w");
  let received = startReceived;
  let lastEmit = 0;
  const speed = new SpeedTracker();
  speed.push(received);
  try {
    for await (const chunk of chunks) {
      received += chunk.byteLength;
      if (received > expected.size) {
        throw new VerificationError(`Received more bytes than expected (${expected.size}).`);
      }
      hash.update(chunk);
      await handle.write(chunk);
      const now = Date.now();
      if (now - lastEmit > 120) {
        lastEmit = now;
        speed.push(received, now);
        onProgress(received, expected.size, speed.bytesPerSecond());
      }
    }
  } finally {
    await handle.close();
  }
  speed.push(received);
  onProgress(received, expected.size, speed.bytesPerSecond());
  if (received !== expected.size) {
    throw new VerificationError(`Size mismatch: expected ${expected.size} bytes, got ${received}.`);
  }
  const digest = hash.digest("hex");
  if (digest !== expected.sha256) {
    throw new VerificationError(`sha256 mismatch: expected ${expected.sha256}, got ${digest}.`);
  }
}

async function* readWithIdleTimeout(
  body: NonNullable<Response["body"]>,
  ctrl: AbortController,
): AsyncIterable<Uint8Array> {
  const reader = body.getReader();
  let timer: NodeJS.Timeout | undefined;
  const arm = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => ctrl.abort(new Error("Download stalled (no data received).")), IDLE_TIMEOUT_MS);
  };
  const onAbort = () => {
    void reader.cancel().catch(() => undefined);
  };
  ctrl.signal.addEventListener("abort", onAbort);
  if (ctrl.signal.aborted) onAbort();
  try {
    arm();
    for (;;) {
      if (ctrl.signal.aborted) {
        const kind = controlKindFromAbort(ctrl.signal);
        throw kind ? new DownloadControlError(kind) : new Error("Download aborted");
      }
      const { done, value } = await reader.read();
      if (done) return;
      arm();
      if (value) yield value;
    }
  } finally {
    ctrl.signal.removeEventListener("abort", onAbort);
    if (timer) clearTimeout(timer);
    try {
      reader.releaseLock();
    } catch {
      /* already cancelled */
    }
  }
}

async function hashFilePrefix(file: string): Promise<{ size: number; hash: ReturnType<typeof createHash> }> {
  const hash = createHash("sha256");
  let size = 0;
  const stream = fs.createReadStream(file, { highWaterMark: 1 << 20 });
  for await (const chunk of stream as AsyncIterable<Buffer>) {
    size += chunk.byteLength;
    hash.update(chunk);
  }
  return { size, hash };
}

async function downloadOnce(
  url: string,
  dest: string,
  expected: { size: number; sha256: string },
  fetchImpl: FetchLike,
  onProgress: TransferProgress,
  ctrl: AbortController,
): Promise<void> {
  let existing = 0;
  let startHash: ReturnType<typeof createHash> | undefined;
  if (fs.existsSync(dest)) {
    const st = fs.statSync(dest);
    if (st.size > 0 && st.size < expected.size) {
      const prefix = await hashFilePrefix(dest);
      existing = prefix.size;
      startHash = prefix.hash;
    } else if (st.size === expected.size) {
      // Complete leftover — verify sha256 then treat as done.
      const prefix = await hashFilePrefix(dest);
      const digest = prefix.hash.digest("hex");
      if (digest !== expected.sha256) {
        fs.rmSync(dest, { force: true });
      } else {
        onProgress(expected.size, expected.size, 0);
        return;
      }
    } else {
      fs.rmSync(dest, { force: true });
    }
  }

  const headers: Record<string, string> = {};
  if (existing > 0) headers.Range = `bytes=${existing}-`;

  let res: Response;
  try {
    res = await fetchImpl(url, { signal: ctrl.signal, redirect: "follow", headers });
  } catch (error) {
    const kind = controlKindFromAbort(ctrl.signal);
    if (kind) throw new DownloadControlError(kind);
    throw error;
  }

  if (existing > 0 && res.status === 206 && res.body) {
    await streamToFile(readWithIdleTimeout(res.body, ctrl), dest, expected, onProgress, {
      append: true,
      startReceived: existing,
      startHash,
    });
    return;
  }

  if (existing > 0 && res.status === 200) {
    console.warn(
      `[litecode] Range resume unsupported for ${url} (HTTP ${res.status}); re-downloading from scratch`,
    );
    fs.rmSync(dest, { force: true });
    existing = 0;
    startHash = undefined;
  }

  if (!res.ok || !res.body) {
    throw new Error(`HTTP ${res.status} for ${url}`);
  }

  if (existing > 0 && res.status !== 206) {
    // Unexpected status with partial file — restart clean.
    fs.rmSync(dest, { force: true });
  }

  await streamToFile(readWithIdleTimeout(res.body, ctrl), dest, expected, onProgress);
}

async function transferPayload(
  location: { kind: "url"; url: string } | { kind: "file"; file: string },
  dest: string,
  expected: { size: number; sha256: string },
  fetchImpl: FetchLike,
  onProgress: TransferProgress,
  ctrl: AbortController,
): Promise<void> {
  if (location.kind === "file") {
    if (!fs.existsSync(location.file)) throw new Error(`Component file missing: ${location.file}`);
    await streamToFile(
      fs.createReadStream(location.file, { highWaterMark: 1 << 20 }),
      dest,
      expected,
      onProgress,
    );
    return;
  }
  let lastError: unknown;
  for (let attempt = 1; attempt <= NETWORK_ATTEMPTS; attempt++) {
    try {
      await downloadOnce(location.url, dest, expected, fetchImpl, onProgress, ctrl);
      return;
    } catch (error) {
      lastError = error;
      const kind = isDownloadControlError(error)
        ? error.kind
        : controlKindFromAbort(ctrl.signal);
      if (kind) {
        if (kind === "stop") fs.rmSync(dest, { force: true });
        throw new DownloadControlError(kind);
      }
      if (error instanceof VerificationError || attempt === NETWORK_ATTEMPTS) {
        fs.rmSync(dest, { force: true });
        break;
      }
      // Keep `.part` so the next attempt can Range-resume.
      await new Promise((r) => setTimeout(r, 800 * attempt));
    }
  }
  throw lastError instanceof Error ? lastError : new Error(String(lastError));
}

/* ------------------------------------------------------------------ */
/* tar.gz extraction (ustar + pax/GNU long names, files and dirs only)  */
/* ------------------------------------------------------------------ */

/** Archive-relative path, or "" for the archive root (`./`). Rejects absolute / `..` paths. */
function safeRelative(name: string): string {
  const cleaned = name.replace(/\\/g, "/");
  if (cleaned.startsWith("/") || /^[A-Za-z]:/.test(cleaned)) {
    throw new Error(`Unsafe path in archive: ${name}`);
  }
  const parts = cleaned.split("/").filter((p) => p && p !== ".");
  if (parts.some((p) => p === "..")) throw new Error(`Unsafe path in archive: ${name}`);
  return parts.join("/");
}

function headerString(block: Buffer, start: number, len: number): string {
  const slice = block.subarray(start, start + len);
  const nul = slice.indexOf(0);
  return slice.subarray(0, nul === -1 ? slice.length : nul).toString("utf8");
}

function headerOctal(block: Buffer, start: number, len: number): number {
  const text = headerString(block, start, len).trim();
  if (!text) return 0;
  const n = Number.parseInt(text, 8);
  if (!Number.isFinite(n) || n < 0) throw new Error("Corrupt tar header (size)");
  return n;
}

function parsePaxPath(body: Buffer): string | null {
  let off = 0;
  let found: string | null = null;
  while (off < body.length) {
    const space = body.indexOf(0x20, off);
    if (space === -1) break;
    const len = Number.parseInt(body.subarray(off, space).toString("ascii"), 10);
    if (!Number.isFinite(len) || len <= 0) break;
    const record = body.subarray(space + 1, off + len - 1).toString("utf8");
    const eq = record.indexOf("=");
    if (eq > 0 && record.slice(0, eq) === "path") found = record.slice(eq + 1);
    off += len;
  }
  return found;
}

/** Extract a gzip'd tar into `destDir`. Only regular files and directories are accepted. */
export async function extractTarGz(archive: string, destDir: string): Promise<string[]> {
  fs.mkdirSync(destDir, { recursive: true });
  const written: string[] = [];
  const stream = zlib.createGunzip();
  // pipeline() forwards read errors into the gunzip stream so the loop below throws.
  pipeline(fs.createReadStream(archive), stream, () => undefined);
  let buf: Buffer = Buffer.alloc(0);
  let body: { remaining: number; pad: number; handle: fs.promises.FileHandle | null; meta: "pax" | "gnu" | null; collected: Buffer[] } | null = null;
  let nextName: string | null = null;
  let ended = false;

  const finishBody = async () => {
    if (!body) return;
    if (body.handle) await body.handle.close();
    if (body.meta) {
      const data = Buffer.concat(body.collected);
      nextName = body.meta === "pax" ? parsePaxPath(data) ?? nextName : headerString(data, 0, data.length);
    }
    body = null;
  };

  try {
    for await (const chunk of stream as AsyncIterable<Buffer>) {
      buf = buf.length ? Buffer.concat([buf, chunk]) : chunk;
      for (;;) {
        if (ended) break;
        if (body) {
          if (body.remaining > 0) {
            if (buf.length === 0) break;
            const take = Math.min(body.remaining, buf.length);
            const piece = buf.subarray(0, take);
            if (body.handle) await body.handle.write(piece);
            else if (body.meta) body.collected.push(Buffer.from(piece));
            body.remaining -= take;
            buf = buf.subarray(take);
            if (body.remaining > 0) break;
          }
          if (buf.length < body.pad) break;
          buf = buf.subarray(body.pad);
          await finishBody();
          continue;
        }
        if (buf.length < 512) break;
        const header = buf.subarray(0, 512);
        buf = buf.subarray(512);
        if (header.every((b) => b === 0)) {
          ended = true;
          break;
        }
        const size = headerOctal(header, 124, 12);
        const pad = (512 - (size % 512)) % 512;
        const type = String.fromCharCode(header[156] || 0x30);
        if (type === "x" || type === "g" || type === "L") {
          body = { remaining: size, pad, handle: null, meta: type === "g" ? null : type === "x" ? "pax" : "gnu", collected: [] };
          if (size === 0) await finishBody();
          continue;
        }
        let name = headerString(header, 0, 100);
        if (headerString(header, 257, 5) === "ustar") {
          const prefix = headerString(header, 345, 155);
          if (prefix) name = `${prefix}/${name}`;
        }
        if (nextName) {
          name = nextName;
          nextName = null;
        }
        const rel = safeRelative(name);
        const target = rel ? path.join(destDir, ...rel.split("/")) : destDir;
        if (type === "5") {
          fs.mkdirSync(target, { recursive: true });
          body = { remaining: size, pad, handle: null, meta: null, collected: [] };
          await finishBody();
          continue;
        }
        if (type !== "0" && type !== "\0" && type !== "7") {
          throw new Error(`Unsupported entry type '${type}' for ${name} in archive`);
        }
        if (!rel) throw new Error(`Unsafe path in archive: ${name}`);
        fs.mkdirSync(path.dirname(target), { recursive: true });
        const handle = await fs.promises.open(target, "w");
        written.push(rel);
        body = { remaining: size, pad, handle, meta: null, collected: [] };
        if (size === 0 && pad === 0) await finishBody();
      }
      if (ended) break;
    }
    if (body) throw new Error("Archive ended in the middle of an entry");
  } finally {
    const open = body as { handle: fs.promises.FileHandle | null } | null;
    if (open?.handle) await open.handle.close().catch(() => undefined);
    stream.destroy();
  }
  return written;
}

/* ------------------------------------------------------------------ */
/* Manager                                                             */
/* ------------------------------------------------------------------ */

export type ComponentLocate = () => string | null;

export type ComponentManagerOptions = {
  appVersion: string;
  /** Existing lookup (env → bundles → sidecar → repo). Non-null = present, skip download. */
  locate: Record<ComponentId, ComponentLocate>;
  emit: (event: ComponentProgressEvent) => void;
  bundlesDir?: string;
  env?: NodeJS.ProcessEnv;
  fetchImpl?: FetchLike;
};

export type EnsureResult = { id: ComponentId; path: string; downloaded: boolean };

export class ComponentManager {
  private readonly inflight = new Map<ComponentId, Promise<EnsureResult>>();
  private readonly lastError = new Map<ComponentId, string>();
  private readonly controllers = new Map<ComponentId, AbortController>();
  private readonly statuses = new Map<ComponentId, ComponentDownloadStatus>();
  private readonly activeParts = new Map<ComponentId, string>();
  private tmpCleaned = false;

  constructor(private readonly opts: ComponentManagerOptions) {
    this.ensureTmpClean();
  }

  private bundlesDir(): string {
    return this.opts.bundlesDir ?? litecodeBundlesDir();
  }

  private tmpDir(): string {
    return path.join(this.bundlesDir(), ".tmp");
  }

  private ensureTmpClean(): void {
    if (this.tmpCleaned) return;
    this.tmpCleaned = true;
    try {
      cleanupComponentTmp(this.tmpDir());
    } catch (error) {
      console.warn("[litecode] component tmp cleanup failed", error);
    }
  }

  private setStatus(id: ComponentId, status: ComponentDownloadStatus): void {
    this.statuses.set(id, status);
  }

  private emitProgress(
    id: ComponentId,
    phase: ComponentPhase,
    status: ComponentDownloadStatus,
    fields: Partial<ComponentProgressEvent> & { message: string },
  ): void {
    this.setStatus(id, status);
    const received = fields.received;
    const total = fields.total;
    this.opts.emit({
      id,
      phase,
      status,
      received,
      total,
      percent: fields.percent ?? progressPercent(received, total),
      bytesPerSecond: fields.bytesPerSecond,
      message: fields.message,
      ...(fields.error ? { error: fields.error } : {}),
    });
  }

  /**
   * Present locally and usable for this app version. A Linux tar that this
   * manager downloaded for a different app version is treated as missing
   * (remote install dir is keyed by version). Files placed by hand are
   * trusted as before.
   */
  locate(id: ComponentId): string | null {
    const found = this.opts.locate[id]();
    if (!found) return null;
    if (id === "linux-server") {
      const bundles = this.bundlesDir();
      const managed = path.resolve(found) === path.resolve(installedLinuxTar(bundles));
      const entry = readInstallRecord(bundles).components[id];
      if (managed && entry && entry.app_version !== this.opts.appVersion) return null;
    }
    return found;
  }

  status(id: ComponentId): {
    id: ComponentId;
    installed: boolean;
    path: string | null;
    downloading: boolean;
    status: ComponentDownloadStatus;
    error: string | null;
  } {
    const found = this.locate(id);
    if (found) this.lastError.delete(id);
    const downloadStatus: ComponentDownloadStatus = found
      ? "done"
      : this.statuses.get(id) ?? (this.inflight.has(id) ? "downloading" : this.lastError.has(id) ? "error" : "idle");
    return {
      id,
      installed: Boolean(found),
      path: found,
      downloading: this.inflight.has(id),
      status: downloadStatus,
      error: found ? null : this.lastError.get(id) ?? null,
    };
  }

  /** Pause in-flight download; keeps `.part` for Range resume. */
  pause(id: ComponentId): boolean {
    const ctrl = this.controllers.get(id);
    if (!ctrl) return false;
    ctrl.abort(new DownloadControlError("pause"));
    return true;
  }

  /** Stop in-flight download and delete `.part` / staging. */
  stop(id: ComponentId): boolean {
    const ctrl = this.controllers.get(id);
    const part = this.activeParts.get(id);
    if (ctrl) ctrl.abort(new DownloadControlError("stop"));
    if (part) {
      fs.rmSync(part, { force: true });
      const staging = part.replace(/\.part$/, "");
      // staging dirs use embed-model-<nonce> naming; best-effort wipe siblings for this id.
      try {
        const dir = path.dirname(part);
        for (const name of fs.readdirSync(dir)) {
          if (name.startsWith(`${id}-`) && !name.endsWith(".part")) {
            fs.rmSync(path.join(dir, name), { recursive: true, force: true });
          }
        }
      } catch {
        /* ignore */
      }
      void staging;
    }
    this.setStatus(id, "stopped");
    return Boolean(ctrl || part);
  }

  /** Re-run ensure (resumes from `.part` when present). */
  retry(id: ComponentId): Promise<EnsureResult> {
    this.lastError.delete(id);
    this.setStatus(id, "idle");
    return this.ensure(id);
  }

  /** Abort every in-flight download (before-quit). Keeps `.part` like pause. */
  abortAll(): void {
    for (const [id, ctrl] of this.controllers) {
      try {
        ctrl.abort(new DownloadControlError("pause"));
      } catch {
        /* ignore */
      }
      this.setStatus(id, "paused");
    }
  }

  /** Return the local path, downloading + installing it first if needed. Concurrent calls share one download. */
  ensure(id: ComponentId): Promise<EnsureResult> {
    this.ensureTmpClean();
    const existing = this.locate(id);
    if (existing) {
      this.lastError.delete(id);
      this.setStatus(id, "done");
      return Promise.resolve({ id, path: existing, downloaded: false });
    }
    const running = this.inflight.get(id);
    if (running) return running;
    const job = this.download(id)
      .then((result) => {
        this.lastError.delete(id);
        this.setStatus(id, "done");
        return result;
      })
      .catch((error: unknown) => {
        if (isDownloadControlError(error)) {
          this.setStatus(id, error.kind === "pause" ? "paused" : "stopped");
          this.opts.emit({
            id,
            phase: "error",
            status: error.kind === "pause" ? "paused" : "stopped",
            message: error.message,
            error: error.message,
          });
          throw error;
        }
        const message = error instanceof Error ? error.message : String(error);
        this.lastError.set(id, message);
        this.setStatus(id, "error");
        this.opts.emit({
          id,
          phase: "error",
          status: "error",
          message: `${COMPONENT_LABEL[id]} download failed`,
          error: message,
        });
        throw new Error(`${COMPONENT_LABEL[id]} download failed: ${message}`);
      })
      .finally(() => {
        this.inflight.delete(id);
        this.controllers.delete(id);
      });
    this.inflight.set(id, job);
    return job;
  }

  private async download(id: ComponentId): Promise<EnsureResult> {
    const { appVersion } = this.opts;
    const label = COMPONENT_LABEL[id];
    const env = this.opts.env ?? process.env;
    const fetchImpl = this.opts.fetchImpl ?? fetch;
    const source = resolveComponentSource(appVersion, env);
    this.emitProgress(id, "resolving", "downloading", { message: `Looking up ${label}…` });
    const manifest = await fetchManifest(source, fetchImpl).catch((error: unknown) => {
      const why = error instanceof Error ? error.message : String(error);
      throw new Error(`${why}\nSource: ${describeSource(source)}\nOffline? Set ${COMPONENTS_BASE_ENV} to a folder containing ${COMPONENTS_MANIFEST_NAME}.`);
    });
    const component = selectComponent(manifest, id, appVersion);
    const location = componentPayloadLocation(source, component);

    const bundles = this.bundlesDir();
    const tmpDir = this.tmpDir();
    fs.mkdirSync(tmpDir, { recursive: true });
    const payload = componentPartPath(tmpDir, id, component.sha256);
    this.activeParts.set(id, payload);
    const ctrl = new AbortController();
    this.controllers.set(id, ctrl);
    const totalMb = (component.size / 1024 / 1024).toFixed(1);
    try {
      this.emitProgress(id, "downloading", "downloading", {
        received: fs.existsSync(payload) ? fs.statSync(payload).size : 0,
        total: component.size,
        bytesPerSecond: 0,
        message: `Downloading ${label} (${totalMb} MB)…`,
      });
      await transferPayload(
        location,
        payload,
        component,
        fetchImpl,
        (received, total, bytesPerSecond) => {
          this.emitProgress(id, "downloading", "downloading", {
            received,
            total,
            bytesPerSecond,
            message: `Downloading ${label} — ${(received / 1024 / 1024).toFixed(1)} / ${totalMb} MB`,
          });
        },
        ctrl,
      );
      this.emitProgress(id, "installing", "downloading", {
        received: component.size,
        total: component.size,
        bytesPerSecond: 0,
        message: `Installing ${label}…`,
      });
      const nonce = randomBytes(6).toString("hex");
      const installed = id === "embed-model"
        ? await this.installModel(payload, component, tmpDir, nonce)
        : this.installLinuxTar(payload, component);
      writeInstallRecord(bundles, id, {
        version: component.version,
        sha256: component.sha256,
        app_version: appVersion,
        installed_at: new Date().toISOString(),
      });
      this.emitProgress(id, "done", "done", {
        received: component.size,
        total: component.size,
        bytesPerSecond: 0,
        message: `${label} ready`,
      });
      return { id, path: installed, downloaded: true };
    } finally {
      this.activeParts.delete(id);
      // Pause keeps `.part` for Range resume; everything else removes leftovers.
      // (linux install renames the part away on success — rm is then a no-op.)
      const kind = controlKindFromAbort(ctrl.signal);
      if (kind !== "pause") {
        try {
          if (fs.existsSync(payload)) fs.rmSync(payload, { force: true });
        } catch {
          /* ignore */
        }
      }
    }
  }

  private async installModel(
    archive: string,
    component: ManifestComponent,
    tmpDir: string,
    nonce: string,
  ): Promise<string> {
    if (component.format !== "tar.gz") throw new Error(`embed-model must be tar.gz, got ${component.format}`);
    const staging = path.join(tmpDir, `embed-model-${nonce}`);
    try {
      await extractTarGz(archive, staging);
      if (!isModelDirReady(staging)) {
        throw new Error("Archive does not contain tokenizer.json, config.json and the ONNX artifact at its root.");
      }
      const finalDir = installedModelDir(this.bundlesDir());
      fs.mkdirSync(path.dirname(finalDir), { recursive: true });
      if (fs.existsSync(finalDir)) {
        // Another window may have finished first; keep a ready install.
        if (isModelDirReady(finalDir)) return finalDir;
        const old = `${finalDir}.old-${nonce}`;
        fs.renameSync(finalDir, old);
        fs.rmSync(old, { recursive: true, force: true });
      }
      fs.renameSync(staging, finalDir);
      return finalDir;
    } finally {
      fs.rmSync(staging, { recursive: true, force: true });
    }
  }

  private installLinuxTar(payload: string, component: ManifestComponent): string {
    if (component.format !== "file") throw new Error(`linux-server must be a plain file, got ${component.format}`);
    const tar = installedLinuxTar(this.bundlesDir());
    fs.mkdirSync(path.dirname(tar), { recursive: true });
    const checksum = `${tar}.sha256`;
    // Write checksum last so resolveLinuxBundle never sees a tar without its pair mid-swap.
    fs.rmSync(checksum, { force: true });
    fs.renameSync(payload, tar);
    fs.writeFileSync(checksum, `${component.sha256}  ${LINUX_TAR_NAME}\n`, "ascii");
    return tar;
  }
}

