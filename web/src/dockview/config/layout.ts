import type { DockviewApi } from "dockview-react";

/** Ensure workspace Search panel exists on the left edge (VS Code Find in Files). */
export function ensureSearchPanel(api: DockviewApi) {
  if (api.getPanel("workspace-search")) return;
  if (!api.getEdgeGroup("left")) {
    api.addEdgeGroup("left", { id: "sidebar-left", initialSize: 280, minimumSize: 180 });
  }
  api.addPanel({
    id: "workspace-search",
    component: "search",
    title: "Search",
    tabComponent: "edge",
    position: { referenceGroup: "sidebar-left" },
  });
}

/**
 * Ensure interactive Terminal panel on the bottom edge. Like the other edge
 * tabs it is part of the default layout: always present and non-closable.
 * Idempotent — does nothing if the panel already exists.
 */
export function ensureTerminalPanel(api: DockviewApi) {
  if (api.getPanel("workspace-terminal")) return;
  ensureBottomEdge(api);
  api.addPanel({
    id: "workspace-terminal",
    component: "terminal",
    title: "Terminal",
    tabComponent: "edge",
    position: { referenceGroup: "sidebar-bottom" },
  });
}

export function buildDefaultLayout(api: DockviewApi) {
  // Left edge: FileTree + Search (persistent sidebar tabs)
  if (!api.getEdgeGroup("left")) {
    api.addEdgeGroup("left", { id: "sidebar-left", initialSize: 280, minimumSize: 180 });
    api.addPanel({
      id: "filetree",
      component: "filetree",
      title: "Explorer",
      tabComponent: "edge",
      position: { referenceGroup: "sidebar-left" },
    });
  }
  ensureSearchPanel(api);

  // Right edge: Session List (persistent sidebar tab)
  if (!api.getEdgeGroup("right")) {
    // `minimumSize` is derived from the panel header's intrinsic min width.
    // With the header count text allowed to truncate, the header can collapse to
    // just the "New" button (~47px) + px-3 padding (24px), so the panel floor can
    // be pushed low enough that session rows shrink to their true minimum
    // (1-char summary + 1-icon live preview). 80px leaves a small buffer.
    api.addEdgeGroup("right", { id: "sidebar-right", initialSize: 320, minimumSize: 20 });
    api.addPanel({
      id: "sessions",
      component: "sessions",
      title: "Sessions",
      tabComponent: "edge",
      position: { referenceGroup: "sidebar-right" },
    });
  }

  // Bottom edge: Terminal (persistent sidebar tab, visible by default,
  // non-closable). It keeps the bottom edge expanded.
  ensureTerminalPanel(api);
}

// Ensure the bottom edge group exists. The Terminal panel lives here permanently
// and the edge is always visible — there is no longer a hidden drop-zone.
export function ensureBottomEdge(api: DockviewApi) {
  if (!api.getEdgeGroup("bottom")) {
    api.addEdgeGroup("bottom", { id: "sidebar-bottom", initialSize: 200, minimumSize: 100 });
  }
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
    position: { referenceGroup: "sidebar-bottom" },
  });
  api.getPanel(id)?.api.setActive();
}



