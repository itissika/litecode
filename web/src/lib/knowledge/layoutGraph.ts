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

/** Canvas dot pitch. Dragging snaps to this, and so does a card that has no saved position. */
export const KNOWLEDGE_GRID = 18;

export function snapKnowledgeCoord(value: number): number {
  return Math.round(value / KNOWLEDGE_GRID) * KNOWLEDGE_GRID;
}

export function knowledgeFolderFlowId(folderId: string): string {
  return `folder:${folderId}`;
}

export interface LaidOutNode {
  id: string;
  nodeId: string;
  /** React Flow parent id. `null` when the node sits on the canvas root. */
  parentId: string | null;
  /** World coordinates. Flow consumers subtract the parent frame. */
  x: number;
  y: number;
}

export interface LaidOutFolder {
  id: string;
  folderId: string;
  parentId: string | null;
  /** World origin of the frame, derived from the nodes inside. */
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

export function knowledgeRelationEdges(
  nodes: KnowledgeNode[],
  issues: KnowledgeIssue[],
): LaidOutEdge[] {
  return collectEdges(nodes, issues);
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

/** Gap between a saved cluster and nodes that still have no world position. */
const FREE_CLUSTER_GAP = 28;

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface GroupLayout {
  origin: { x: number; y: number };
  width: number;
  height: number;
  anchored: boolean;
  nodes: LaidOutNode[];
  folders: LaidOutFolder[];
}

function cardBox(node: KnowledgeNode): { width: number; height: number } {
  const width =
    node.w != null && Number.isFinite(node.w) && node.w > 0
      ? node.w
      : KNOWLEDGE_NODE_WIDTH;
  const height =
    node.h != null && Number.isFinite(node.h) && node.h > 0
      ? node.h
      : KNOWLEDGE_NODE_HEIGHT;
  return { width, height };
}

function hasWorldPosition(
  node: KnowledgeNode,
): node is KnowledgeNode & { x: number; y: number } {
  return (
    node.x != null &&
    node.y != null &&
    Number.isFinite(node.x) &&
    Number.isFinite(node.y)
  );
}

function boundsOf(rects: Rect[]): {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
} {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const rect of rects) {
    minX = Math.min(minX, rect.x);
    minY = Math.min(minY, rect.y);
    maxX = Math.max(maxX, rect.x + rect.w);
    maxY = Math.max(maxY, rect.y + rect.h);
  }
  return { minX, minY, maxX, maxY };
}

/** Folder frame around child world rects. Matches the live folder fit. */
function frameAround(rects: Rect[]): {
  x: number;
  y: number;
  width: number;
  height: number;
} {
  const { minX, minY, maxX, maxY } = boundsOf(rects);
  return {
    x: minX - KNOWLEDGE_FOLDER_PAD,
    y: minY - (KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD),
    width: maxX - minX + KNOWLEDGE_FOLDER_PAD * 2,
    height:
      KNOWLEDGE_FOLDER_HEADER + (maxY - minY) + KNOWLEDGE_FOLDER_PAD * 2,
  };
}

function shiftGroup(layout: GroupLayout, dx: number, dy: number): GroupLayout {
  if (dx === 0 && dy === 0) return layout;
  return {
    ...layout,
    origin: { x: layout.origin.x + dx, y: layout.origin.y + dy },
    nodes: layout.nodes.map((node) => ({
      ...node,
      x: node.x + dx,
      y: node.y + dy,
    })),
    folders: layout.folders.map((folder) => ({
      ...folder,
      x: folder.x + dx,
      y: folder.y + dy,
    })),
  };
}

function emptyGroup(): GroupLayout {
  return {
    origin: { x: 0, y: 0 },
    width: KNOWLEDGE_NODE_WIDTH,
    height: KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_NODE_HEIGHT,
    anchored: false,
    nodes: [],
    folders: [],
  };
}

function layoutContainer(
  folderId: string | null,
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
  edges: LaidOutEdge[],
  seen: Set<string>,
): GroupLayout {
  if (folderId != null) {
    if (seen.has(folderId)) return emptyGroup();
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

  const parentKey = folderId == null ? null : knowledgeFolderFlowId(folderId);
  const anchoredNodes = childNodes.filter(hasWorldPosition);
  const freeNodes = childNodes.filter((node) => !hasWorldPosition(node));
  const anchoredNested = nested.filter((item) => item.layout.anchored);
  const freeNested = nested.filter((item) => !item.layout.anchored);

  const fixedRects: Rect[] = [
    ...anchoredNodes.map((node) => {
      const box = cardBox(node);
      return { x: node.x, y: node.y, w: box.width, h: box.height };
    }),
    ...anchoredNested.map((item) => ({
      x: item.layout.origin.x,
      y: item.layout.origin.y,
      w: item.layout.width,
      h: item.layout.height,
    })),
  ];

  const freeBoxes: Box[] = [
    ...freeNodes.map((node) => {
      const box = cardBox(node);
      return { key: `n:${node.id}`, width: box.width, height: box.height };
    }),
    ...freeNested.map((item) => ({
      key: `f:${item.folder.id}`,
      width: item.layout.width,
      height: item.layout.height,
    })),
  ];
  const placed = placeBoxes(
    freeBoxes,
    edges.map((edge) => ({
      source: `n:${edge.source}`,
      target: `n:${edge.target}`,
    })),
  );

  let freeOrigin = { x: 0, y: 0 };
  if (fixedRects.length > 0 && freeBoxes.length > 0) {
    const fixed = boundsOf(fixedRects);
    freeOrigin = { x: fixed.maxX + FREE_CLUSTER_GAP, y: fixed.minY };
  } else if (fixedRects.length === 0 && folderId != null) {
    freeOrigin = {
      x: KNOWLEDGE_FOLDER_PAD,
      y: KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD,
    };
  }

  const directNodes: LaidOutNode[] = [
    ...anchoredNodes.map((node) => ({
      id: node.id,
      nodeId: node.id,
      parentId: parentKey,
      x: node.x,
      y: node.y,
    })),
    ...freeNodes.map((node) => {
      const pos = placed.get(`n:${node.id}`) ?? { x: 0, y: 0 };
      return {
        id: node.id,
        nodeId: node.id,
        parentId: parentKey,
        x: freeOrigin.x + pos.x,
        y: freeOrigin.y + pos.y,
      };
    }),
  ];

  const placedNested = [
    ...anchoredNested,
    ...freeNested.map((item) => {
      const pos = placed.get(`f:${item.folder.id}`) ?? { x: 0, y: 0 };
      return {
        folder: item.folder,
        layout: shiftGroup(
          item.layout,
          freeOrigin.x + pos.x - item.layout.origin.x,
          freeOrigin.y + pos.y - item.layout.origin.y,
        ),
      };
    }),
  ];

  const sizeById = new Map(
    childNodes.map((node) => [node.id, cardBox(node)] as const),
  );
  const childRects: Rect[] = [
    ...directNodes.map((node) => {
      const box = sizeById.get(node.id) ?? {
        width: KNOWLEDGE_NODE_WIDTH,
        height: KNOWLEDGE_NODE_HEIGHT,
      };
      return { x: node.x, y: node.y, w: box.width, h: box.height };
    }),
    ...placedNested.map((item) => ({
      x: item.layout.origin.x,
      y: item.layout.origin.y,
      w: item.layout.width,
      h: item.layout.height,
    })),
  ];

  const framed =
    folderId != null && childRects.length > 0 ? frameAround(childRects) : null;
  const content = childRects.length > 0 ? boundsOf(childRects) : null;
  const origin = framed ?? { x: 0, y: 0 };
  const width = framed?.width ?? content?.maxX ?? KNOWLEDGE_NODE_WIDTH;
  const height =
    framed?.height ??
    content?.maxY ??
    (folderId == null
      ? KNOWLEDGE_NODE_HEIGHT
      : KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_NODE_HEIGHT);

  const directFolders: LaidOutFolder[] = placedNested.map((item) => ({
    id: knowledgeFolderFlowId(item.folder.id),
    folderId: item.folder.id,
    parentId: parentKey,
    x: item.layout.origin.x,
    y: item.layout.origin.y,
    width: item.layout.width,
    height: item.layout.height,
  }));

  return {
    origin,
    width,
    height,
    anchored: fixedRects.length > 0,
    nodes: [
      ...directNodes,
      ...placedNested.flatMap((item) => item.layout.nodes),
    ],
    folders: [
      ...directFolders,
      ...placedNested.flatMap((item) => item.layout.folders),
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
 * Node positions are world coordinates. A folder frame is the bounds of the
 * nodes inside it, and is not stored.
 */
export function layoutKnowledgeGraph(
  nodes: KnowledgeNode[],
  issues: KnowledgeIssue[],
  folders: KnowledgeFolder[] = [],
): { nodes: LaidOutNode[]; folders: LaidOutFolder[]; edges: LaidOutEdge[] } {
  const edges = collectEdges(nodes, issues);
  if (folders.length === 0) {
    const aligned = alignUnsaved(layoutFlat(nodes, edges), [], nodes);
    return { nodes: aligned.nodes, folders: [], edges };
  }
  const grouped = layoutContainer(null, nodes, folders, edges, new Set<string>());
  const aligned = alignUnsaved(grouped.nodes, grouped.folders, nodes);
  return { nodes: aligned.nodes, folders: aligned.folders, edges };
}

/** Cards with no saved position land on the grid. Saved coordinates stay put. */
function alignUnsaved(
  laid: LaidOutNode[],
  folders: LaidOutFolder[],
  source: KnowledgeNode[],
): { nodes: LaidOutNode[]; folders: LaidOutFolder[] } {
  const saved = new Set(source.filter(hasWorldPosition).map((node) => node.id));
  const nodes = laid.map((node) =>
    saved.has(node.nodeId)
      ? node
      : {
          ...node,
          x: snapKnowledgeCoord(node.x),
          y: snapKnowledgeCoord(node.y),
        },
  );
  if (folders.length === 0) return { nodes, folders };
  return { nodes, folders: reframeFolders(folders, nodes, source) };
}

function reframeFolders(
  folders: LaidOutFolder[],
  nodes: LaidOutNode[],
  source: KnowledgeNode[],
): LaidOutFolder[] {
  const sizeById = new Map(source.map((node) => [node.id, cardBox(node)] as const));
  const byId = new Map(folders.map((folder) => [folder.id, folder]));
  const depthOf = (id: string | null): number => {
    let depth = 0;
    const seen = new Set<string>();
    let current = id;
    while (current && !seen.has(current)) {
      seen.add(current);
      depth += 1;
      current = byId.get(current)?.parentId ?? null;
    }
    return depth;
  };
  const next = new Map(byId);
  const deepestFirst = [...folders].sort((a, b) => depthOf(b.id) - depthOf(a.id));
  for (const folder of deepestFirst) {
    const childNodes = nodes.filter((node) => node.parentId === folder.id);
    const childFolders = folders
      .filter((item) => item.parentId === folder.id)
      .map((item) => next.get(item.id))
      .filter((item): item is LaidOutFolder => item != null);
    const rects: Rect[] = [
      ...childNodes.map((node) => {
        const box = sizeById.get(node.nodeId) ?? {
          width: KNOWLEDGE_NODE_WIDTH,
          height: KNOWLEDGE_NODE_HEIGHT,
        };
        return { x: node.x, y: node.y, w: box.width, h: box.height };
      }),
      ...childFolders.map((item) => ({
        x: item.x,
        y: item.y,
        w: item.width,
        h: item.height,
      })),
    ];
    if (rects.length === 0) continue;
    const frame = frameAround(rects);
    next.set(folder.id, { ...folder, ...frame });
  }
  return folders.map((folder) => next.get(folder.id) ?? folder);
}
