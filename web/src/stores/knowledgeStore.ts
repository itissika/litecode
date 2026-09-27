import { create } from "zustand";

import { knowledgeFixture } from "../lib/knowledge/fixture";
import { normalizeKey } from "../lib/knowledge/markers";
import type { KnowledgeIssue, KnowledgeNode } from "../lib/knowledge/types";
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

const indexed = indexNodes(knowledgeFixture);

interface KnowledgeStore {
  nodes: KnowledgeNode[];
  byId: Map<number, KnowledgeNode>;
  byKey: Map<string, KnowledgeNode>;
  issues: KnowledgeIssue[];
  issuesByNode: Map<number, KnowledgeIssue[]>;
  expanded: Set<number>;
  focusedId: number | null;
  /** Bumps on every focus so a repeated jump still scrolls and flashes. */
  focusNonce: number;
  focus: (id: number) => void;
  toggle: (id: number) => void;
  expandAll: () => void;
  collapseAll: () => void;
}

export const useKnowledgeStore = create<KnowledgeStore>((set) => ({
  ...indexed,
  expanded: new Set<number>(),
  focusedId: null,
  focusNonce: 0,
  focus: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      const expanded = new Set(state.expanded);
      expanded.add(id);
      return { focusedId: id, focusNonce: state.focusNonce + 1, expanded };
    }),
  toggle: (id) =>
    set((state) => {
      const expanded = new Set(state.expanded);
      if (expanded.has(id)) expanded.delete(id);
      else expanded.add(id);
      return { expanded };
    }),
  expandAll: () =>
    set((state) => ({
      expanded: new Set(state.nodes.map((node) => node.id)),
    })),
  collapseAll: () => set({ expanded: new Set() }),
}));
