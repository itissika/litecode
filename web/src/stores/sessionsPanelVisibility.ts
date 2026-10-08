import { useSyncExternalStore } from "react";

import { panelVisible, watchPanelVisible } from "../dockview/workbench/queries";

/**
 * Tracks the visibility of the dockview `sessions` panel (the session list).
 * Used as the PRIMARY trigger gate for the live-summary placeholder emoji:
 * an emoji may only pop in while the user can actually see the list.
 *
 * Implemented as a module-level external store so every `LivePreview` shares a
 * single subscription instead of each row wiring its own dockview listener.
 */

const SESSIONS_PANEL_ID = "sessions";

let visible = false;
let unwatch: (() => void) | undefined;
const listeners = new Set<() => void>();

function recompute(): void {
  const next = panelVisible(SESSIONS_PANEL_ID);
  if (next === visible) return;
  visible = next;
  listeners.forEach((listener) => listener());
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  if (listeners.size === 1) unwatch = watchPanelVisible(SESSIONS_PANEL_ID, recompute);
  return () => {
    listeners.delete(cb);
    if (listeners.size === 0) {
      unwatch?.();
      unwatch = undefined;
    }
  };
}

function getSnapshot(): boolean {
  return visible;
}

export function useSessionsPanelVisible(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
