import type { AddPanelOptions, DockviewApi, DockviewGroupPanel } from "dockview-react";

import { isMainGrid } from "../../dockview/popout/location";
import { isWorkspaceFileRef } from "./markers";
import { resolveCitation, type Citation } from "./refDisplay";
import { getDockviewApi } from "../../stores/connectionStore";
import { externalPreviewId, useEditorStore } from "../../stores/editorStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { fileNameFromPath } from "../../utils/language";

export const KNOWLEDGE_GRAPH_PANEL_ID = "knowledge-graph";

type DockviewApiLive = NonNullable<ReturnType<typeof getDockviewApi>>;

/** A brand-new grid group beside the current one, so existing panels stay put. */
function besideCurrentGrid(api: DockviewApiLive): {
  referenceGroup: string;
  direction?: "right";
} {
  const gridGroups = api.groups.filter((group) => isMainGrid(group.api.location.type));
  if (gridGroups.length === 0) {
    const group = api.addGroup();
    return { referenceGroup: group.id };
  }
  const active = api.activeGroup;
  const anchor =
    active && isMainGrid(active.api.location.type) ? active : gridGroups[0]!;
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

const recentGridGroups = new WeakMap<DockviewApi, string[]>();
const gridGroupWatch = new WeakMap<DockviewApi, true>();

function rememberGridGroup(api: DockviewApi, group: DockviewGroupPanel | undefined): void {
  if (!group || !isMainGrid(group.api.location.type)) return;
  const recent = recentGridGroups.get(api) ?? [];
  const id = group.api.id;
  const next = recent.filter((item) => item !== id);
  next.push(id);
  recentGridGroups.set(api, next.slice(-12));
}

/** Dockview has no cross-group MRU. `onDidActiveGroupChange` is the record it does publish. */
/** Start recording center-grid group activation. Safe to call more than once. */
export function watchGridGroups(api: DockviewApi): void {
  if (gridGroupWatch.has(api)) return;
  gridGroupWatch.set(api, true);
  if (typeof api.onDidActiveGroupChange === "function") {
    api.onDidActiveGroupChange((group) => rememberGridGroup(api, group));
  }
  rememberGridGroup(api, api.activeGroup);
}

function trackGridGroups(api: DockviewApi): string[] {
  watchGridGroups(api);
  return recentGridGroups.get(api) ?? [];
}

function gridGroupsExcept(api: DockviewApi, sourceId: string | null) {
  return api.groups.filter(
    (group) => isMainGrid(group.api.location.type) && group.api.id !== sourceId,
  );
}

/**
 * Where a citation panel goes.
 * An existing panel is activated in place. A new one joins the other center
 * grid group when there is one, and only splits a group when the center has
 * nowhere else to put it.
 */
export function placeGridPanel(
  api: DockviewApi,
  panel: Pick<AddPanelOptions, "id" | "component" | "title" | "tabComponent" | "params">,
): void {
  const existing = api.getPanel(panel.id);
  if (existing) {
    existing.api.setActive();
    return;
  }
  const recent = trackGridGroups(api);
  const source =
    api.activeGroup && isMainGrid(api.activeGroup.api.location.type)
      ? api.activeGroup
      : undefined;
  const others = gridGroupsExcept(api, source?.api.id ?? null);
  let position: AddPanelOptions["position"];
  if (others.length === 0) {
    position = source
      ? { referenceGroup: source.api.id, direction: "right" }
      : { direction: "right" };
  } else if (others.length === 1) {
    position = { referenceGroup: others[0]!.api.id };
  } else {
    const otherIds = new Set(others.map((group) => group.api.id));
    let chosen = others[others.length - 1]!.api.id;
    for (let index = recent.length - 1; index >= 0; index -= 1) {
      const id = recent[index];
      if (id && otherIds.has(id) && id !== source?.api.id) {
        chosen = id;
        break;
      }
    }
    position = { referenceGroup: chosen };
  }
  api.addPanel({ ...panel, position });
}

function viewingKnowledgeGraph(api: DockviewApi): boolean {
  const panel = api.activePanel;
  if (!panel) return false;
  return panel.id === KNOWLEDGE_GRAPH_PANEL_ID || panel.api.component === "knowledgeGraph";
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
    const api = getDockviewApi();
    if (api && viewingKnowledgeGraph(api)) {
      useKnowledgeStore.getState().focusCanvas(model.targetId);
      return;
    }
    if (api) {
      placeGridPanel(api, {
        id: KNOWLEDGE_GRAPH_PANEL_ID,
        component: "knowledgeGraph",
        title: "Knowledge Graph",
        tabComponent: "knowledgeGraph",
      });
    }
    useKnowledgeStore.getState().focusCanvas(model.targetId);
    return;
  }
  const api = getDockviewApi();
  const preview = api?.getPanel(externalPreviewId(citation.path));
  if (preview) {
    preview.api.setActive();
    return;
  }
  if (!isWorkspaceFileRef(citation.path)) return;
  if (api) {
    placeGridPanel(api, {
      id: citation.path,
      component: "editor",
      title: fileNameFromPath(citation.path),
      tabComponent: "editor",
      params: { filePath: citation.path },
    });
  }
  const editor = useEditorStore.getState();
  if (citation.line != null) void editor.openFileAt(citation.path, citation.line);
  else void editor.openFile(citation.path);
}
