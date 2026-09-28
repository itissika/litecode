import { create } from "zustand";

import {
  knowledgeFixture,
  knowledgeFolderFixture,
} from "../lib/knowledge/fixture";
import { normalizeKey } from "../lib/knowledge/markers";
import type {
  KnowledgeFolder,
  KnowledgeIssue,
  KnowledgeNode,
} from "../lib/knowledge/types";
import { groupIssues, validateKnowledge } from "../lib/knowledge/validate";

function indexNodes(nodes: KnowledgeNode[]) {
  const byId = new Map<number, KnowledgeNode>();
  const byKey = new Map<string, KnowledgeNode>();
  for (const node of nodes) {
    if (!byId.has(node.id)) byId.set(node.id, node);
    const key = normalizeKey(node.key);
    if (key && !byKey.has(key)) byKey.set(key, node);
  }
  const issues = validateKnowledge(nodes);
  return { nodes, byId, byKey, issues, issuesByNode: groupIssues(issues) };
}

function indexFolders(folders: KnowledgeFolder[]) {
  const folderById = new Map<number, KnowledgeFolder>();
  for (const folder of folders) {
    if (!folderById.has(folder.id)) folderById.set(folder.id, folder);
  }
  return { folders, folderById };
}

const indexed = {
  ...indexNodes(knowledgeFixture),
  ...indexFolders(knowledgeFolderFixture),
};

interface KnowledgeStore {
  nodes: KnowledgeNode[];
  byId: Map<number, KnowledgeNode>;
  byKey: Map<string, KnowledgeNode>;
  folders: KnowledgeFolder[];
  folderById: Map<number, KnowledgeFolder>;
  issues: KnowledgeIssue[];
  issuesByNode: Map<number, KnowledgeIssue[]>;
  expanded: Set<number>;
  expandedFolders: Set<number>;
  graphExpanded: Set<number>;
  focusedId: number | null;
  /** Bumps when the canvas should pan to `focusedId` (side body / ref jump). */
  focusNonce: number;
  /** Side-list flash target when a graph card is clicked. */
  flashId: number | null;
  flashNonce: number;
  /** Pan the graph to this node; does not expand the side row. */
  focusCanvas: (id: number) => void;
  /** Graph card click: highlight graph + flash side without expanding or panning. */
  selectFromGraph: (id: number) => void;
  /** Click graph background — drop canvas/side focus highlight. */
  clearCanvasFocus: () => void;
  toggle: (id: number) => void;
  toggleFolder: (id: number) => void;
  toggleGraph: (id: number) => void;
  expandAll: () => void;
  collapseAll: () => void;
}

export const useKnowledgeStore = create<KnowledgeStore>((set) => ({
  ...indexed,
  expanded: new Set<number>(),
  expandedFolders: new Set(knowledgeFolderFixture.map((folder) => folder.id)),
  graphExpanded: new Set<number>(),
  focusedId: null,
  focusNonce: 0,
  flashId: null,
  flashNonce: 0,
  focusCanvas: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      return {
        focusedId: id,
        focusNonce: state.focusNonce + 1,
      };
    }),
  selectFromGraph: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      return {
        focusedId: id,
        flashId: id,
        flashNonce: state.flashNonce + 1,
      };
    }),
  clearCanvasFocus: () =>
    set((state) => {
      if (state.focusedId == null) return state;
      return { focusedId: null, flashId: null };
    }),
  toggle: (id) =>
    set((state) => {
      const expanded = new Set(state.expanded);
      if (expanded.has(id)) expanded.delete(id);
      else expanded.add(id);
      return { expanded };
    }),
  toggleFolder: (id) =>
    set((state) => {
      if (!state.folderById.has(id)) return state;
      const expandedFolders = new Set(state.expandedFolders);
      if (expandedFolders.has(id)) expandedFolders.delete(id);
      else expandedFolders.add(id);
      return { expandedFolders };
    }),
  toggleGraph: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      const graphExpanded = new Set(state.graphExpanded);
      if (graphExpanded.has(id)) graphExpanded.delete(id);
      else graphExpanded.add(id);
      return { graphExpanded };
    }),
  expandAll: () =>
    set((state) => ({
      expanded: new Set(state.nodes.map((node) => node.id)),
    })),
  collapseAll: () => set({ expanded: new Set() }),
}));
