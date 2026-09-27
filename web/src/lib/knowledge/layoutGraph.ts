import dagre from "@dagrejs/dagre";

import { normalizeKey } from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";

export const KNOWLEDGE_NODE_WIDTH = 200;
export const KNOWLEDGE_NODE_HEIGHT = 84;

export interface LaidOutNode {
  id: string;
  nodeId: number;
  x: number;
  y: number;
}

export interface LaidOutEdge {
  id: string;
  source: string;
  target: string;
  warning: boolean;
}

function isWarning(
  issues: KnowledgeIssue[],
  sourceId: number,
  targetKey: string,
): boolean {
  return issues.some(
    (issue) =>
      issue.nodeId === sourceId &&
      (issue.code === "unused_relation" || issue.code === "inactive_target") &&
      issue.ref === targetKey,
  );
}

/** Left-to-right layout. Self links and relations with no target are omitted. */
export function layoutKnowledgeGraph(
  nodes: KnowledgeNode[],
  issues: KnowledgeIssue[],
): { nodes: LaidOutNode[]; edges: LaidOutEdge[] } {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const edges: LaidOutEdge[] = [];
  const seen = new Set<string>();
  for (const node of nodes) {
    for (const rel of node.relations) {
      if (rel === node.id || !byId.has(rel)) continue;
      const id = `${node.id}-${rel}`;
      if (seen.has(id)) continue;
      seen.add(id);
      const target = byId.get(rel)!;
      edges.push({
        id,
        source: String(node.id),
        target: String(rel),
        warning: isWarning(issues, node.id, normalizeKey(target.key)),
      });
    }
  }

  const graph = new dagre.graphlib.Graph();
  graph.setDefaultEdgeLabel(() => ({}));
  graph.setGraph({
    rankdir: "LR",
    nodesep: 18,
    ranksep: 56,
    marginx: 16,
    marginy: 16,
  });
  for (const node of nodes) {
    graph.setNode(String(node.id), {
      width: KNOWLEDGE_NODE_WIDTH,
      height: KNOWLEDGE_NODE_HEIGHT,
    });
  }
  for (const edge of edges) graph.setEdge(edge.source, edge.target);
  dagre.layout(graph);

  const laidOut: LaidOutNode[] = [];
  for (const node of nodes) {
    const pos = graph.node(String(node.id)) as { x: number; y: number };
    laidOut.push({
      id: String(node.id),
      nodeId: node.id,
      x: pos.x - KNOWLEDGE_NODE_WIDTH / 2,
      y: pos.y - KNOWLEDGE_NODE_HEIGHT / 2,
    });
  }
  return { nodes: laidOut, edges };
}
