import type { DockviewApi } from "dockview-react";

import { dockview } from "./host";
import { edgeKinds, kindForComponent, type EdgeRail } from "./kinds";

const EDGE_OPTS: Record<
  EdgeRail,
  { id: string; initialSize: number; minimumSize: number }
> = {
  left: { id: "sidebar-left", initialSize: 280, minimumSize: 180 },
  right: { id: "sidebar-right", initialSize: 320, minimumSize: 20 },
  bottom: { id: "sidebar-bottom", initialSize: 200, minimumSize: 100 },
};

function liveApi(api?: DockviewApi | null): DockviewApi | null {
  return api ?? dockview();
}

function ensureEdge(api: DockviewApi, rail: EdgeRail): string {
  const existing = api.getEdgeGroup(rail);
  if (existing) return existing.id;
  return api.addEdgeGroup(rail, EDGE_OPTS[rail]).id;
}

/** Create the singleton edge panel for a kind when it is missing. */
export function ensureEdgePanel(component: string, api?: DockviewApi | null): void {
  const live = liveApi(api);
  const spec = kindForComponent(component);
  if (!live || !spec?.singletonId || spec.zone !== "edge" || !spec.rail) return;
  if (live.getPanel(spec.singletonId)) return;
  live.addPanel({
    id: spec.singletonId,
    component: spec.component,
    title: spec.title ?? spec.component,
    tabComponent: spec.tabComponent,
    position: { referenceGroup: ensureEdge(live, spec.rail) },
  });
}

/** Show an edge panel, creating it when the rail lost it. */
export function revealEdgePanel(component: string): boolean {
  const live = dockview();
  if (!live) return false;
  ensureEdgePanel(component, live);
  const spec = kindForComponent(component);
  const panel = spec?.singletonId ? live.getPanel(spec.singletonId) : undefined;
  if (!panel) return false;
  panel.api.group?.api.expand?.();
  panel.api.setActive();
  return true;
}

function removeLegacyTerminalPanels(api: DockviewApi): void {
  const extras = api.panels.filter((panel) =>
    panel.id.startsWith("workspace-terminal-"),
  );
  for (const panel of extras) {
    try {
      panel.api.close();
    } catch {
      // The panel may already be gone.
    }
  }
}

export function ensureDefaultEdges(api?: DockviewApi | null): void {
  const live = liveApi(api);
  if (!live) return;
  for (const kind of edgeKinds()) ensureEdgePanel(kind.component, live);
  removeLegacyTerminalPanels(live);
}

function hasRequiredRails(api: DockviewApi): boolean {
  return !!(
    api.getPanel("filetree") &&
    api.getPanel("sessions") &&
    api.getPanel("workspace-terminal")
  );
}

/**
 * Repair a broken workspace chrome without throwing. Prefer re-adding missing
 * default panels; if an edge is empty and its required panel is gone, recreate
 * that edge; last resort is `clear()` plus the three rails.
 */
export function recoverDefaultLayout(api?: DockviewApi | null): void {
  const live = liveApi(api);
  if (!live) return;
  try {
    ensureDefaultEdges(live);
    if (hasRequiredRails(live)) return;

    for (const rail of ["left", "right", "bottom"] as const) {
      const required =
        rail === "left" ? "filetree" : rail === "right" ? "sessions" : "workspace-terminal";
      if (live.getPanel(required)) continue;
      try {
        if (live.getEdgeGroup(rail)) live.removeEdgeGroup(rail);
      } catch {
        // Recreate the edge below.
      }
    }
    ensureDefaultEdges(live);
    if (hasRequiredRails(live)) return;

    live.clear();
    ensureDefaultEdges(live);
  } catch {
    try {
      live.clear();
    } catch {
      // Ignore a second failure and still try to rebuild the rails.
    }
    ensureDefaultEdges(live);
  }
}
