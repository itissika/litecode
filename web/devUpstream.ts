import * as fs from "node:fs";

/** Stand-in target when the desktop host has not published a sidecar yet. */
export const DEV_UPSTREAM_CLOSED = "http://127.0.0.1:9";

/** Loopback only: the dev proxy must not be pointed at an arbitrary host. */
function isLoopbackDevHost(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return host === "127.0.0.1" || host === "localhost" || host === "::1";
}

/**
 * Sidecar origin published by the Electron host, or null when the file is
 * missing, empty, or not a loopback http(s) URL.
 *
 * The desktop sidecar binds `127.0.0.1:0`, so the port is unknown until READY
 * and changes again when a workspace relaunches it. Vite reads this file on
 * each proxied request instead of a bind fixed at startup.
 */
export function readDevUpstream(filePath: string): string | null {
  let text: string;
  try {
    text = fs.readFileSync(filePath, "utf8");
  } catch {
    return null;
  }
  return loopbackOrigin(text);
}

function loopbackOrigin(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  if (url.username || url.password) return null;
  if (!isLoopbackDevHost(url.hostname)) return null;
  return url.origin;
}

/** Update a live proxy options object. Empty file points at a closed port. */
export function pointProxyAtDevUpstream(
  options: { target?: unknown },
  filePath: string,
): boolean {
  const next = readDevUpstream(filePath);
  options.target = next ?? DEV_UPSTREAM_CLOSED;
  return next !== null;
}
