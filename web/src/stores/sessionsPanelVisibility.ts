import { useSyncExternalStore } from "react";
import { getDockviewApi } from "./connectionStore";

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
let wired = false;
const listeners = new Set<() => void>();

function recompute(): void {
  const v = getDockviewApi()?.getPanel(SESSIONS_PANEL_ID)?.api.isVisible ?? false;
  if (v !== visible) {
    visible = v;
    listeners.forEach((l) => l());
  }
}

function wire(): void {
  if (wired) return;
  const tryWire = (): void => {
    const api = getDockviewApi();
    if (!api) {
      // Dockview not mounted yet — retry shortly.
      window.setTimeout(tryWire, 250);
      return;
    }
    const panel = api.getPanel(SESSIONS_PANEL_ID);
    panel?.api.onDidVisibilityChange(recompute);
    // Re-check if the layout (and thus the panel) is rebuilt.
    api.onDidLayoutChange?.(recompute);
    recompute();
    wired = true;
  };
  tryWire();
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  if (listeners.size === 1) wire();
  return () => {
    listeners.delete(cb);
  };
}

function getSnapshot(): boolean {
  return visible;
}

export function useSessionsPanelVisible(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
