import fs from "node:fs";
import path from "node:path";

/**
 * Dev-only workbench document. `dev_win.ps1` sets this to the Vite origin so
 * the desktop window hot-reloads `web/` while Electron still owns the sidecar.
 * Absent in packaged builds and in `-NoHmr`.
 */
export function devUiDocumentUrl(raw: string | undefined | null): string | null {
  const trimmed = raw?.trim() ?? "";
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    throw new Error(`LITECODE_UI_DEV_URL is not a URL: ${trimmed}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error(`LITECODE_UI_DEV_URL must be http(s): ${trimmed}`);
  }
  if (url.username || url.password) {
    throw new Error("LITECODE_UI_DEV_URL must not include credentials");
  }
  if (!isLoopbackDevHost(url.hostname)) {
    throw new Error(`LITECODE_UI_DEV_URL must be a loopback address: ${trimmed}`);
  }
  if ((url.pathname !== "/" && url.pathname !== "") || url.search || url.hash) {
    throw new Error("LITECODE_UI_DEV_URL must be an origin, without a path, query, or hash");
  }
  return `${url.origin}/`;
}

export function devUpstreamFilePath(env: NodeJS.ProcessEnv = process.env): string | null {
  const file = env.LITECODE_DEV_UPSTREAM_FILE?.trim();
  return file ? file : null;
}

/** Vite can follow the sidecar only when both the document URL and the file are set. */
export function assertDevUiConfigured(uiUrl: string | null, upstreamFile: string | null): void {
  if (uiUrl && !upstreamFile) {
    throw new Error(
      "LITECODE_UI_DEV_URL requires LITECODE_DEV_UPSTREAM_FILE so the Vite proxy can follow the sidecar port.",
    );
  }
}

/** Loopback only. A published upstream is an open proxy target for the dev server. */
export function isLoopbackDevHost(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return host === "127.0.0.1" || host === "localhost" || host === "::1";
}

export function sidecarDevOrigin(readyUrl: string): string {
  let url: URL;
  try {
    url = new URL(readyUrl);
  } catch {
    throw new Error(`Sidecar READY URL is not a URL: ${readyUrl}`);
  }
  if ((url.protocol !== "http:" && url.protocol !== "https:") || url.username || url.password) {
    throw new Error(`Refusing to publish sidecar URL: ${readyUrl}`);
  }
  if (!isLoopbackDevHost(url.hostname)) {
    throw new Error(`Refusing to publish a non-loopback sidecar URL: ${readyUrl}`);
  }
  return url.origin;
}

/**
 * Tell Vite which sidecar to proxy to. `readyUrl` null clears the file so a
 * stopped sidecar is not reused. No-op when the dev loop did not set the path.
 */
export function syncDevUpstream(
  readyUrl: string | null,
  env: NodeJS.ProcessEnv = process.env,
): void {
  const file = devUpstreamFilePath(env);
  if (!file) return;
  const body = readyUrl ? sidecarDevOrigin(readyUrl) : "";
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body, "utf8");
}

/** True when `target` is the document already loaded (query and hash ignored). */
export function sameWorkbenchDocument(current: string, target: string): boolean {
  if (!current || !target) return false;
  try {
    const left = new URL(current);
    const right = new URL(target);
    return left.origin === right.origin && left.pathname === right.pathname;
  } catch {
    return false;
  }
}
