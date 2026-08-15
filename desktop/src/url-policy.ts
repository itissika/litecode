/**
 * Protocol whitelists for Electron URL sinks (G5).
 *
 * - `openExternal` (window.open / external links) may only open http(s)/mailto.
 * - `loadURL` (in-window navigation) may only load http(s).
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
