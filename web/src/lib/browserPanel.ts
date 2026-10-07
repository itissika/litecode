import type { DockviewApi, IDockviewGroupPanel } from "dockview-react";

import { editorPanelPosition } from "../stores/editorStore";
import { isMainGrid } from "../dockview/popout/location";

export type BrowserBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserPanelState = {
  id: string;
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
};

const SCHEME = /^[a-zA-Z][a-zA-Z+\-.]*:/;
const MAX_URL_LENGTH = 8000;

/** Address-bar text. A fresh page has no URL yet. */
export function displayBrowserUrl(url: string): string {
  if (!url || url === "about:blank") return "";
  return url;
}

/** Accept a bare host as https. Refuse every scheme other than http(s). */
export function normalizeBrowserUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > MAX_URL_LENGTH) return null;
  const withScheme = SCHEME.test(trimmed) ? trimmed : `https://${trimmed}`;
  try {
    const url = new URL(withScheme);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    return url.toString();
  } catch {
    return null;
  }
}

export function sameBrowserUrl(a: string, b: string): boolean {
  try {
    return new URL(a).toString() === new URL(b).toString();
  } catch {
    return a === b;
  }
}

/** The + lives on grid tab bars only. Edge rails stay as they are. */
export function showBrowserAddButton(
  locationType: string | undefined,
  hasBridge: boolean,
): boolean {
  return hasBridge && isMainGrid(locationType);
}

export const MAIN_BROWSER_PLACE = "main";

/**
 * Rectangle target for the native page. A mismatch means the element has
 * moved to another window (or the view has not caught up). Callers must hide
 * the page instead of forwarding that rectangle.
 */
export function browserBoundsPlace(
  ownerIsMainDocument: boolean,
  place: string,
): string | null {
  if (ownerIsMainDocument && place === MAIN_BROWSER_PLACE) return MAIN_BROWSER_PLACE;
  if (!ownerIsMainDocument && place !== MAIN_BROWSER_PLACE) return place;
  return null;
}

function isAgentGroup(group: IDockviewGroupPanel): boolean {
  return group.panels.some(
    (panel) =>
      panel.api.component === "agent" || panel.api.component === "subagent",
  );
}

/**
 * Add a browser tab. A normal editor group receives it directly. An agent
 * group keeps its own strip, and the page opens with the editors instead.
 */
export function addBrowserPanel(api: DockviewApi, preferredGroupId?: string): void {
  const id = `browser-${crypto.randomUUID()}`;
  const preferred = preferredGroupId ? api.getGroup(preferredGroupId) : undefined;
  const position =
    preferred &&
    isMainGrid(preferred.api.location.type) &&
    !isAgentGroup(preferred)
      ? { referenceGroup: preferred.api.id }
      : editorPanelPosition(api);
  api.addPanel({
    id,
    component: "browser",
    title: "Browser",
    tabComponent: "browser",
    params: { url: "" },
    position,
  });
}
