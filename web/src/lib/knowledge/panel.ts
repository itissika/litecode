import { openPanel, revealPanel } from "../../dockview/workbench/commands";
import { activePanelComponent, activePanelId, hasPanel } from "../../dockview/workbench/queries";
import { isWorkspaceFileRef } from "./markers";
import { resolveCitation, type Citation } from "./refDisplay";
import { externalPreviewId, useEditorStore } from "../../stores/editorStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

export const KNOWLEDGE_GRAPH_PANEL_ID = "knowledge-graph";

/** Show the knowledge graph on the main center. */
export function openKnowledgeGraphPanel(): void {
  openPanel({
    id: KNOWLEDGE_GRAPH_PANEL_ID,
    component: "knowledgeGraph",
    title: "Knowledge Graph",
    tabComponent: "knowledgeGraph",
  });
}

/** Focus one node and make sure the graph panel is on screen. */
export function revealKnowledgeNode(id: string): void {
  useKnowledgeStore.getState().focusCanvas(id);
  openKnowledgeGraphPanel();
}

function viewingKnowledgeGraph(): boolean {
  return (
    activePanelId() === KNOWLEDGE_GRAPH_PANEL_ID ||
    activePanelComponent() === "knowledgeGraph"
  );
}

function citationInput(citation: Citation, sourceId?: string) {
  const knowledge = useKnowledgeStore.getState();
  const source = sourceId ? (knowledge.byId.get(sourceId) ?? null) : null;
  const key = citation.kind === "node" ? citation.key.trim() : "";
  const target = key ? (knowledge.byKey.get(key) ?? null) : null;
  const issues = sourceId ? (knowledge.issuesByNode.get(sourceId) ?? []) : [];
  const externalOpen =
    citation.kind === "file" &&
    useEditorStore
      .getState()
      .tabs.some((tab) => tab.external && tab.path === externalPreviewId(citation.path));
  return { source, target, issues, externalOpen };
}

/**
 * Follow one citation.
 * Knowledge chips already on the graph only focus the card. Every other
 * resolvable citation asks the target panel to show itself.
 */
export function followCitation(citation: Citation, sourceId?: string): void {
  const model = resolveCitation(citation, citationInput(citation, sourceId));
  if (!model.resolvable) return;
  if (citation.kind === "node") {
    if (!model.targetId) return;
    if (!viewingKnowledgeGraph()) openKnowledgeGraphPanel();
    useKnowledgeStore.getState().focusCanvas(model.targetId);
    return;
  }
  const previewId = externalPreviewId(citation.path);
  if (hasPanel(previewId)) {
    revealPanel(previewId);
    return;
  }
  if (!isWorkspaceFileRef(citation.path)) return;
  const editor = useEditorStore.getState();
  if (citation.line != null) void editor.openFileAt(citation.path, citation.line);
  else void editor.openFile(citation.path);
}
