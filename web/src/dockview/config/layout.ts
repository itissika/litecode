import type { DockviewApi } from "dockview-react";

const LEFT_EDGE_ID = "sidebar-left";
const RIGHT_EDGE_ID = "sidebar-right";
const BOTTOM_EDGE_ID = "sidebar-bottom";

type EdgePos = "left" | "right" | "bottom";

const EDGE_OPTS: Record<
  EdgePos,
  { id: string; initialSize: number; minimumSize: number }
> = {
  left: { id: LEFT_EDGE_ID, initialSize: 280, minimumSize: 180 },
  right: { id: RIGHT_EDGE_ID, initialSize: 320, minimumSize: 20 },
  bottom: { id: BOTTOM_EDGE_ID, initialSize: 200, minimumSize: 100 },
};

/** Return the live edge-group id, creating the group if it is missing. */
export function ensureEdge(api: DockviewApi, pos: EdgePos): string {
  const existing = api.getEdgeGroup(pos);
  if (existing) return existing.id;
  return api.addEdgeGroup(pos, EDGE_OPTS[pos]).id;
}

/** Ensure the left edge group exists (Explorer / Search / Source Control rail). */
export function ensureLeftEdge(api: DockviewApi) {
  ensureEdge(api, "left");
}

/** Ensure the right edge group exists (Sessions rail). */
export function ensureRightEdge(api: DockviewApi) {
  ensureEdge(api, "right");
}

/** Ensure the bottom edge group exists. The Terminal panel lives here permanently. */
export function ensureBottomEdge(api: DockviewApi) {
  ensureEdge(api, "bottom");
}

/** Ensure the Explorer panel exists on the left edge. */
export function ensureExplorerPanel(api: DockviewApi) {
  if (api.getPanel("filetree")) return;
  api.addPanel({
    id: "filetree",
    component: "filetree",
    title: "Explorer",
    tabComponent: "edge",
    position: { referenceGroup: ensureEdge(api, "left") },
  });
}

/** Ensure workspace Search panel exists on the left edge (VS Code Find in Files). */
export function ensureSearchPanel(api: DockviewApi) {
  if (api.getPanel("workspace-search")) return;
  api.addPanel({
    id: "workspace-search",
    component: "search",
    title: "Search",
    tabComponent: "edge",
    position: { referenceGroup: ensureEdge(api, "left") },
  });
}

/** Ensure Source Control panel exists on the left edge (VS Code SCM). */
export function ensureGitPanel(api: DockviewApi) {
  if (api.getPanel("workspace-git")) return;
  api.addPanel({
    id: "workspace-git",
    component: "git",
    title: "Source Control",
    tabComponent: "edge",
    position: { referenceGroup: ensureEdge(api, "left") },
  });
}

/** Ensure the Sessions panel exists on the right edge. */
export function ensureSessionsPanel(api: DockviewApi) {
  if (api.getPanel("sessions")) return;
  api.addPanel({
    id: "sessions",
    component: "sessions",
    title: "Sessions",
    tabComponent: "edge",
    position: { referenceGroup: ensureEdge(api, "right") },
  });
}

/** Ensure the interactive Terminal panel on the bottom edge. */
export function ensureTerminalPanel(api: DockviewApi) {
  if (api.getPanel("workspace-terminal")) return;
  api.addPanel({
    id: "workspace-terminal",
    component: "terminal",
    title: "Terminal",
    tabComponent: "edge",
    position: { referenceGroup: ensureEdge(api, "bottom") },
  });
}

/**
 * Ensure every default-layout panel exists. Idempotent — safe to call after a
 * layout restore. This is the "no side panel goes missing" guarantee: when a
 * version update adds a new default panel, or the persisted snapshot predates
 * it, the missing panel is re-added here without discarding the user's layout.
 * Hidden panels still exist (getPanel returns them), so a user's choice to
 * hide a panel is respected.
 */
export function ensureDefaultPanels(api: DockviewApi) {
  ensureExplorerPanel(api);
  ensureSearchPanel(api);
  ensureGitPanel(api);
  ensureSessionsPanel(api);
  ensureTerminalPanel(api);
}

export function buildDefaultLayout(api: DockviewApi) {
  ensureDefaultPanels(api);
}

function hasRequiredRails(api: DockviewApi): boolean {
  return !!(
    api.getPanel("filetree") &&
    api.getPanel("sessions") &&
    api.getPanel("workspace-terminal")
  );
}

/**
 * Repair a broken workspace chrome without throwing. Used after layout restore
 * and after exceptional panel closes (e.g. a persisted agent tab whose session
 * no longer exists). Prefer re-adding missing default panels; if an edge is
 * empty and its required panel is gone, recreate that edge; last resort is
 * `clear()` + the three-rail default.
 */
export function recoverDefaultLayout(api: DockviewApi): void {
  try {
    ensureDefaultPanels(api);
    if (hasRequiredRails(api)) return;

    for (const pos of ["left", "right", "bottom"] as const) {
      const required =
        pos === "left"
          ? "filetree"
          : pos === "right"
            ? "sessions"
            : "workspace-terminal";
      if (api.getPanel(required)) continue;
      try {
        if (api.getEdgeGroup(pos)) api.removeEdgeGroup(pos);
      } catch {
        // ignore — recreate below
      }
    }
    ensureDefaultPanels(api);
    if (hasRequiredRails(api)) return;

    api.clear();
    buildDefaultLayout(api);
  } catch {
    try {
      api.clear();
    } catch {
      // ignore
    }
    buildDefaultLayout(api);
  }
}

let extraTerminalSeq = 0;

/** Open an additional integrated terminal tab rooted at a workspace-relative cwd. */
export function openTerminalAt(api: DockviewApi, cwd: string) {
  extraTerminalSeq += 1;
  const id = `workspace-terminal-${extraTerminalSeq}`;
  const leaf = cwd.split("/").filter(Boolean).pop();
  const title = leaf ? `Terminal · ${leaf}` : "Terminal";
  api.addPanel({
    id,
    component: "terminal",
    title,
    tabComponent: "edge",
    params: { cwd },
    position: { referenceGroup: ensureEdge(api, "bottom") },
  });
  api.getPanel(id)?.api.setActive();
}
