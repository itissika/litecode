import { getDockviewApi } from "../../stores/connectionStore";
import { gridPosition } from "../sessionPanelNav";

export const KNOWLEDGE_GRAPH_PANEL_ID = "knowledge-graph";

/** Open the single graph panel, or focus it when it is already open. */
export function openKnowledgeGraphPanel(): void {
  const api = getDockviewApi();
  if (!api) return;
  const existing = api.getPanel(KNOWLEDGE_GRAPH_PANEL_ID);
  if (existing) {
    existing.api.setActive();
    return;
  }
  api.addPanel({
    id: KNOWLEDGE_GRAPH_PANEL_ID,
    component: "knowledgeGraph",
    title: "Knowledge Graph",
    tabComponent: "knowledgeGraph",
    position: gridPosition(api),
  });
}
