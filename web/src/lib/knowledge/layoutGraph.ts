import dagre from "@dagrejs/dagre";

import { normalizeKey } from "./markers";
import type { KnowledgeFolder, KnowledgeIssue, KnowledgeNode } from "./types";

export const KNOWLEDGE_NODE_WIDTH = 200;
export const KNOWLEDGE_NODE_HEIGHT = 84;
export const KNOWLEDGE_NODE_MIN_WIDTH = 140;
export const KNOWLEDGE_NODE_MIN_HEIGHT = 64;
export const KNOWLEDGE_NODE_MAX_WIDTH = 480;
export const KNOWLEDGE_NODE_MAX_HEIGHT = 560;

export const KNOWLEDGE_FOLDER_HEADER = 28;
export const KNOWLEDGE_FOLDER_PAD = 16;

export function knowledgeFolderFlowId(folderId: string): string {
  return `folder:${folderId}`;
}

export interface LaidOutNode {
  id: string;
  nodeId: string;
  /** React Flow parent id. `null` when the node sits on the canvas root. */
  parentId: string | null;
  x: number;
  y: number;
}

export interface LaidOutFolder {
  id: string;
  folderId: string;
  parentId: string | null;
  x: number;
  y: number;
  width: number;
  height: number;
}

export type KnowledgeEdgeVariant = "solid" | "unused" | "inactive";

export interface LaidOutEdge {
  id: string;
  source: string;
  target: string;
  variant: KnowledgeEdgeVariant;
}

function edgeVariant(
  issues: KnowledgeIssue[],
  sourceId: string,
  targetKey: string,
): KnowledgeEdgeVariant {
  const inactive = issues.some(
    (issue) =>
      issue.nodeId === sourceId &&
      issue.code === "inactive_target" &&
      issue.ref === targetKey,
  );
  if (inactive) return "inactive";
  return "solid";
}

function collectEdges(
  nodes: KnowledgeNode[],
  issues: KnowledgeIssue[],
): LaidOutEdge[] {
  const byKey = new Map<string, KnowledgeNode>();
  for (const node of nodes) {
    const key = normalizeKey(node.key);
    if (key && !byKey.has(key)) byKey.set(key, node);
  }
  const edges: LaidOutEdge[] = [];
  const seen = new Set<string>();
  for (const node of nodes) {
    const ownKey = normalizeKey(node.key);
    for (const rel of node.relations) {
      if (rel === ownKey) continue;
      const target = byKey.get(rel);
      if (!target) continue;
      const id = `${node.id}-${target.id}`;
      if (seen.has(id)) continue;
      seen.add(id);
      edges.push({
        id,
        source: node.id,
        target: target.id,
        variant: edgeVariant(issues, node.id, normalizeKey(target.key)),
      });
    }
  }
  return edges;
}

interface Box {
  key: string;
  width: number;
  height: number;
}

function placeBoxes(
  boxes: Box[],
  edges: Array<{ source: string; target: string }>,
): Map<string, { x: number; y: number }> {
  const placed = new Map<string, { x: number; y: number }>();
  if (boxes.length === 0) return placed;
  const graph = new dagre.graphlib.Graph();
  graph.setDefaultEdgeLabel(() => ({}));
  graph.setGraph({
    rankdir: "LR",
    nodesep: 28,
    ranksep: 40,
    marginx: 0,
    marginy: 0,
  });
  for (const box of boxes) {
    graph.setNode(box.key, { width: box.width, height: box.height });
  }
  const keys = new Set(boxes.map((box) => box.key));
  for (const edge of edges) {
    if (keys.has(edge.source) && keys.has(edge.target)) {
      graph.setEdge(edge.source, edge.target);
    }
  }
  dagre.layout(graph);
  let minX = Infinity;
  let minY = Infinity;
  const raw: Array<{ key: string; x: number; y: number }> = [];
  for (const box of boxes) {
    const pos = graph.node(box.key) as { x: number; y: number };
    const x = pos.x - box.width / 2;
    const y = pos.y - box.height / 2;
    raw.push({ key: box.key, x, y });
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
  }
  for (const item of raw) {
    placed.set(item.key, { x: item.x - minX, y: item.y - minY });
  }
  return placed;
}

interface GroupLayout {
  width: number;
  height: number;
  nodes: LaidOutNode[];
  folders: LaidOutFolder[];
}

function layoutContainer(
  folderId: string | null,
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
  edges: LaidOutEdge[],
  seen: Set<string>,
): GroupLayout {
  if (folderId != null) {
    if (seen.has(folderId)) {
      return {
        width: KNOWLEDGE_NODE_WIDTH,
        height: KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_NODE_HEIGHT,
        nodes: [],
        folders: [],
      };
    }
    seen.add(folderId);
  }

  const folderIds = new Set(folders.map((folder) => folder.id));
  const childFolders = folders.filter((folder) => {
    const parent = folder.parentId ?? null;
    const resolved = parent != null && folderIds.has(parent) ? parent : null;
    return resolved === folderId;
  });
  const childNodes = nodes.filter((node) => {
    const parent = node.folderId ?? null;
    const resolved = parent != null && folderIds.has(parent) ? parent : null;
    return resolved === folderId;
  });
  const nested = childFolders.map((folder) => ({
    folder,
    layout: layoutContainer(folder.id, nodes, folders, edges, seen),
  }));

  const boxes: Box[] = [
    ...childNodes.map((node) => ({
      key: `n:${node.id}`,
      width: KNOWLEDGE_NODE_WIDTH,
      height: KNOWLEDGE_NODE_HEIGHT,
    })),
    ...nested.map((item) => ({
      key: `f:${item.folder.id}`,
      width: item.layout.width,
      height: item.layout.height,
    })),
  ];
  const placed = placeBoxes(
    boxes,
    edges.map((edge) => ({
      source: `n:${edge.source}`,
      target: `n:${edge.target}`,
    })),
  );

  const parentKey = folderId == null ? null : knowledgeFolderFlowId(folderId);
  const chrome = folderId == null ? 0 : KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD;
  const pad = folderId == null ? 0 : KNOWLEDGE_FOLDER_PAD;

  let contentWidth = 0;
  let contentHeight = 0;
  const directNodes: LaidOutNode[] = [];
  for (const node of childNodes) {
    const pos = placed.get(`n:${node.id}`) ?? { x: 0, y: 0 };
    const x = node.x ?? pos.x + pad;
    const y = node.y ?? pos.y + chrome;
    contentWidth = Math.max(contentWidth, x + KNOWLEDGE_NODE_WIDTH);
    contentHeight = Math.max(contentHeight, y + KNOWLEDGE_NODE_HEIGHT);
    directNodes.push({
      id: node.id,
      nodeId: node.id,
      parentId: parentKey,
      x,
      y,
    });
  }

  const directFolders: LaidOutFolder[] = [];
  for (const item of nested) {
    const pos = placed.get(`f:${item.folder.id}`) ?? { x: 0, y: 0 };
    contentWidth = Math.max(contentWidth, pos.x + item.layout.width);
    contentHeight = Math.max(contentHeight, pos.y + item.layout.height);
    directFolders.push({
      id: knowledgeFolderFlowId(item.folder.id),
      folderId: item.folder.id,
      parentId: parentKey,
      x: pos.x + pad,
      y: pos.y + chrome,
      width: item.layout.width,
      height: item.layout.height,
    });
  }

  if (boxes.length === 0) {
    contentWidth = KNOWLEDGE_NODE_WIDTH;
    contentHeight = KNOWLEDGE_NODE_HEIGHT;
  }

  const width =
    folderId == null
      ? contentWidth
      : Math.max(contentWidth + pad, KNOWLEDGE_NODE_WIDTH) + pad;
  const height =
    folderId == null
      ? contentHeight
      : chrome + contentHeight + pad;

  return {
    width,
    height,
    nodes: [...directNodes, ...nested.flatMap((item) => item.layout.nodes)],
    folders: [
      ...directFolders,
      ...nested.flatMap((item) => item.layout.folders),
    ],
  };
}

function layoutFlat(
  nodes: KnowledgeNode[],
  edges: LaidOutEdge[],
): LaidOutNode[] {
  const placed = placeBoxes(
    nodes.map((node) => ({
      key: node.id,
      width: KNOWLEDGE_NODE_WIDTH,
      height: KNOWLEDGE_NODE_HEIGHT,
    })),
    edges,
  );
  return nodes.map((node) => {
    const pos = placed.get(node.id) ?? { x: 0, y: 0 };
    return {
      id: node.id,
      nodeId: node.id,
      parentId: null,
      x: node.x ?? pos.x + 16,
      y: node.y ?? pos.y + 16,
    };
  });
}

/**
 * Left-to-right layout. Self links and relations with no target are omitted.
 * Folders are frames only: nested folders and their nodes are placed inside,
 * with positions relative to the parent frame.
 */
export function layoutKnowledgeGraph(
  nodes: KnowledgeNode[],
  issues: KnowledgeIssue[],
  folders: KnowledgeFolder[] = [],
): { nodes: LaidOutNode[]; folders: LaidOutFolder[]; edges: LaidOutEdge[] } {
  const edges = collectEdges(nodes, issues);
  if (folders.length === 0) {
    return { nodes: layoutFlat(nodes, edges), folders: [], edges };
  }
  const grouped = layoutContainer(null, nodes, folders, edges, new Set<string>());
  return { nodes: grouped.nodes, folders: grouped.folders, edges };
}
