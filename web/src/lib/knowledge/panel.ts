import { getDockviewApi } from "../../stores/connectionStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

export const KNOWLEDGE_GRAPH_PANEL_ID = "knowledge-graph";

type DockviewApi = NonNullable<ReturnType<typeof getDockviewApi>>;

/** A brand-new grid group beside the current one, so existing panels stay put. */
function besideCurrentGrid(api: DockviewApi): {
  referenceGroup: string;
  direction?: "right";
} {
  const gridGroups = api.groups.filter((group) => group.api.location.type === "grid");
  if (gridGroups.length === 0) {
    const group = api.addGroup();
    return { referenceGroup: group.id };
  }
  const active = api.activeGroup;
  const anchor =
    active && active.api.location.type === "grid" ? active : gridGroups[0]!;
  return { referenceGroup: anchor.api.id, direction: "right" };
}

/**
 * Show the knowledge graph.
 * Missing: split a new grid group; never drop a tab onto the current panel.
 * Present but hidden: activate that panel.
 */
export function openKnowledgeGraphPanel(): void {
  const api = getDockviewApi();
  if (!api) return;
  const existing = api.getPanel(KNOWLEDGE_GRAPH_PANEL_ID);
  if (existing) {
    if (!existing.api.isVisible) existing.api.setActive();
    return;
  }
  api.addPanel({
    id: KNOWLEDGE_GRAPH_PANEL_ID,
    component: "knowledgeGraph",
    title: "Knowledge Graph",
    tabComponent: "knowledgeGraph",
    position: besideCurrentGrid(api),
  });
}

/** Focus one node and make sure the graph panel is on screen. */
export function revealKnowledgeNode(id: string): void {
  useKnowledgeStore.getState().focusCanvas(id);
  openKnowledgeGraphPanel();
}
