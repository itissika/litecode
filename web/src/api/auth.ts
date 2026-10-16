/** Host-injected or dev-only auth token for local serve. Never a user settings field. */

// Single source: desktop/src/bridge-contract.ts (type-only; no runtime import).
import type {
  ComponentProgressEvent,
  LitecodeDesktopBridge,
} from "../../../desktop/src/bridge-contract";

export type { ComponentProgressEvent, LitecodeDesktopBridge };

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
function withAuthHeaders(init?: HeadersInit): Headers {
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
