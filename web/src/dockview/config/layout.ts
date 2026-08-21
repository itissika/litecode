import type { DockviewApi } from "dockview-react";

const LEFT_EDGE_ID = "sidebar-left";
const RIGHT_EDGE_ID = "sidebar-right";
const BOTTOM_EDGE_ID = "sidebar-bottom";

/** Ensure the left edge group exists (Explorer / Search / Source Control rail). */
export function ensureLeftEdge(api: DockviewApi) {
  if (!api.getEdgeGroup("left")) {
    api.addEdgeGroup("left", { id: LEFT_EDGE_ID, initialSize: 280, minimumSize: 180 });
  }
}

/** Ensure the right edge group exists (Sessions rail). */
export function ensureRightEdge(api: DockviewApi) {
  if (!api.getEdgeGroup("right")) {
    api.addEdgeGroup("right", { id: RIGHT_EDGE_ID, initialSize: 320, minimumSize: 20 });
  }
}

/** Ensure the bottom edge group exists. The Terminal panel lives here permanently. */
export function ensureBottomEdge(api: DockviewApi) {
  if (!api.getEdgeGroup("bottom")) {
    api.addEdgeGroup("bottom", { id: BOTTOM_EDGE_ID, initialSize: 200, minimumSize: 100 });
  }
}

/** Ensure the Explorer panel exists on the left edge. */
export function ensureExplorerPanel(api: DockviewApi) {
  if (api.getPanel("filetree")) return;
  ensureLeftEdge(api);
  api.addPanel({
    id: "filetree",
    component: "filetree",
    title: "Explorer",
    tabComponent: "edge",
    position: { referenceGroup: LEFT_EDGE_ID },
  });
}

/** Ensure workspace Search panel exists on the left edge (VS Code Find in Files). */
export function ensureSearchPanel(api: DockviewApi) {
  if (api.getPanel("workspace-search")) return;
  ensureLeftEdge(api);
  api.addPanel({
    id: "workspace-search",
    component: "search",
    title: "Search",
    tabComponent: "edge",
    position: { referenceGroup: LEFT_EDGE_ID },
  });
}

/** Ensure Source Control panel exists on the left edge (VS Code SCM). */
export function ensureGitPanel(api: DockviewApi) {
  if (api.getPanel("workspace-git")) return;
  ensureLeftEdge(api);
  api.addPanel({
    id: "workspace-git",
    component: "git",
    title: "Source Control",
    tabComponent: "edge",
    position: { referenceGroup: LEFT_EDGE_ID },
  });
}

/** Ensure the Sessions panel exists on the right edge. */
export function ensureSessionsPanel(api: DockviewApi) {
  if (api.getPanel("sessions")) return;
  ensureRightEdge(api);
  api.addPanel({
    id: "sessions",
    component: "sessions",
    title: "Sessions",
    tabComponent: "edge",
    position: { referenceGroup: RIGHT_EDGE_ID },
  });
}

/** Ensure the interactive Terminal panel on the bottom edge. */
export function ensureTerminalPanel(api: DockviewApi) {
  if (api.getPanel("workspace-terminal")) return;
  ensureBottomEdge(api);
  api.addPanel({
    id: "workspace-terminal",
    component: "terminal",
    title: "Terminal",
    tabComponent: "edge",
    position: { referenceGroup: BOTTOM_EDGE_ID },
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

let extraTerminalSeq = 0;

/** Open an additional integrated terminal tab rooted at a workspace-relative cwd. */
export function openTerminalAt(api: DockviewApi, cwd: string) {
  ensureBottomEdge(api);
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
    position: { referenceGroup: BOTTOM_EDGE_ID },
  });
  api.getPanel(id)?.api.setActive();
}
