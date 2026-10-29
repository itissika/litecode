/**
 * Protocol and origin whitelists for Electron URL sinks (G5).
 *
 * - `openExternal` (window.open / external links) may only open http(s)/mailto.
 * - `loadURL` (in-window navigation) may only load http(s).
 * - local app views additionally pin every top-level load to the app's origin.
 *
 * Any other scheme (file:, data:, javascript:, custom://…) is refused so a
 * compromised renderer or untrusted link cannot reach local files or drive
 * other windows.
 */

export function isAllowedExternalUrl(raw: string): boolean {
  let protocol: string;
  try {
    protocol = new URL(raw).protocol;
  } catch {
    return false;
  }
  return protocol === "http:" || protocol === "https:" || protocol === "mailto:";
}

export function isAllowedLoadUrl(raw: string): boolean {
  let protocol: string;
  try {
    protocol = new URL(raw).protocol;
  } catch {
    return false;
  }
  return protocol === "http:" || protocol === "https:";
}

/** Origin without path/query, or null for a non-http(s) / malformed URL. */
export function httpOrigin(raw: string): string | null {
  try {
    const url = new URL(raw);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    return url.origin;
  } catch {
    return null;
  }
}

export function sameHttpOrigin(left: string, right: string): boolean {
  const a = httpOrigin(left);
  const b = httpOrigin(right);
  return a !== null && a === b;
}

function loopbackHost(hostname: string): boolean {
  return hostname === "127.0.0.1" || hostname === "localhost" || hostname === "[::1]";
}

/** An app may redirect between equivalent loopback spellings on the same port. */
export function sameAppHttpOrigin(left: string, right: string): boolean {
  if (sameHttpOrigin(left, right)) return true;
  try {
    const a = new URL(left);
    const b = new URL(right);
    return (
      a.protocol === "http:" &&
      b.protocol === "http:" &&
      loopbackHost(a.hostname) &&
      loopbackHost(b.hostname) &&
      a.port === b.port
    );
  } catch {
    return false;
  }
}

/** App processes must be loopback HTTP; never a remote host or file URL. */
export function isLoopbackHttpUrl(raw: string): boolean {
  try {
    const url = new URL(raw);
    if (url.protocol !== "http:") return false;
    if (url.username || url.password) return false;
    return (
      url.hostname === "127.0.0.1" ||
      url.hostname === "localhost" ||
      url.hostname === "[::1]" ||
      url.hostname === "::1"
    );
  } catch {
    return false;
  }
}
