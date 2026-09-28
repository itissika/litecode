import {
  KNOWLEDGE_FOLDER_HEADER,
  KNOWLEDGE_FOLDER_PAD,
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_WIDTH,
} from "./layoutGraph";

/** Ignore sub-pixel jitter so a fitted frame does not refit itself. */
const FIT_EPSILON = 1;

export interface FitNode {
  id: string;
  type?: string;
  parentId?: string;
  position: { x: number; y: number };
  width?: number | null;
  height?: number | null;
  measured?: { width?: number | null; height?: number | null };
  style?: { width?: number | string; height?: number | string };
}

function cssDimension(value: number | string | null | undefined, fallback: number): number {
  if (typeof value === "number" && Number.isFinite(value) && value > 0) return value;
  if (typeof value === "string") {
    const parsed = Number.parseFloat(value);
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
  }
  return fallback;
}

function nodeSize(node: FitNode): { w: number; h: number } {
  const fallbackW = node.type === "knowledgeFolder" ? 0 : KNOWLEDGE_NODE_WIDTH;
  const fallbackH = node.type === "knowledgeFolder" ? 0 : KNOWLEDGE_NODE_HEIGHT;
  return {
    w: cssDimension(
      node.measured?.width ?? node.width ?? node.style?.width,
      fallbackW,
    ),
    h: cssDimension(
      node.measured?.height ?? node.height ?? node.style?.height,
      fallbackH,
    ),
  };
}

function parentDepth(id: string, parentOf: Map<string, string | undefined>): number {
  let depth = 0;
  let current = parentOf.get(id);
  const seen = new Set<string>();
  while (current && !seen.has(current)) {
    seen.add(current);
    depth += 1;
    current = parentOf.get(current);
  }
  return depth;
}

/**
 * Resize each folder so it wraps its direct children with stable padding.
 * Innermost folders update first; each ancestor then wraps the new box.
 * Child positions shift with the folder origin so absolute placement stays put.
 */
export function fitKnowledgeFolders<T extends FitNode>(nodes: T[]): T[] {
  const folders = nodes.filter((node) => node.type === "knowledgeFolder");
  if (folders.length === 0) return nodes;

  const parentOf = new Map(nodes.map((node) => [node.id, node.parentId]));
  const ordered = [...folders].sort(
    (a, b) => parentDepth(b.id, parentOf) - parentDepth(a.id, parentOf),
  );

  const next = nodes.slice();
  const indexOf = new Map(next.map((node, index) => [node.id, index]));
  let changed = false;

  for (const folder of ordered) {
    const folderIndex = indexOf.get(folder.id);
    if (folderIndex == null) continue;
    const currentFolder = next[folderIndex];
    const childIndexes: number[] = [];
    for (let index = 0; index < next.length; index += 1) {
      if (next[index].parentId === currentFolder.id) childIndexes.push(index);
    }
    if (childIndexes.length === 0) continue;

    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const index of childIndexes) {
      const child = next[index];
      const { w, h } = nodeSize(child);
      if (w <= 0 || h <= 0) continue;
      minX = Math.min(minX, child.position.x);
      minY = Math.min(minY, child.position.y);
      maxX = Math.max(maxX, child.position.x + w);
      maxY = Math.max(maxY, child.position.y + h);
    }
    if (!Number.isFinite(minX) || !Number.isFinite(minY)) continue;

    const deltaX = minX - KNOWLEDGE_FOLDER_PAD;
    const deltaY = minY - (KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD);
    const width = maxX - minX + KNOWLEDGE_FOLDER_PAD * 2;
    const height =
      KNOWLEDGE_FOLDER_HEADER + (maxY - minY) + KNOWLEDGE_FOLDER_PAD * 2;
    const { w: prevW, h: prevH } = nodeSize(currentFolder);
    if (
      Math.abs(deltaX) < FIT_EPSILON &&
      Math.abs(deltaY) < FIT_EPSILON &&
      Math.abs(prevW - width) < FIT_EPSILON &&
      Math.abs(prevH - height) < FIT_EPSILON
    ) {
      continue;
    }

    changed = true;
    next[folderIndex] = {
      ...currentFolder,
      position: {
        x: currentFolder.position.x + deltaX,
        y: currentFolder.position.y + deltaY,
      },
      width,
      height,
      measured: { width, height },
      style: { ...currentFolder.style, width, height },
    };
    for (const index of childIndexes) {
      const child = next[index];
      next[index] = {
        ...child,
        position: {
          x: child.position.x - deltaX,
          y: child.position.y - deltaY,
        },
      };
    }
  }

  return changed ? next : nodes;
}
