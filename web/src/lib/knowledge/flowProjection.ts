import type { LaidOutEdge } from "./layoutGraph";
import { normalizeKey } from "./markers";
import type { KnowledgeIssue } from "./types";

/** Membership and parentage. Summary, body, and coordinates stay out. */
export function knowledgeStructureKey(
  nodes: readonly { id: string; path: string; folderId?: string | null }[],
  folders: readonly { id: string; parentId: string | null }[],
): string {
  let key = "";
  for (const folder of folders) {
    key += `f:${folder.id}:${folder.parentId ?? ""}\n`;
  }
  for (const node of nodes) {
    key += `n:${node.id}:${node.path}:${node.folderId ?? ""}\n`;
  }
  return key;
}

/** Outgoing citations and declaration keys. Status and summary stay out. */
export function relationSignature(
  nodes: readonly { id: string; key: string; relations: readonly string[] }[],
): string {
  let key = "";
  for (const node of nodes) {
    key += `${node.id}=${normalizeKey(node.key)}>${node.relations.join(",")};`;
  }
  return key;
}

/** Declaration keys, in corpus order. Used as `@` candidates. */
export function mentionKeysOf(nodes: readonly { key: string }[]): string {
  let key = "";
  for (const node of nodes) {
    const normalized = normalizeKey(node.key);
    if (!normalized) continue;
    if (key) key += "\n";
    key += normalized;
  }
  return key;
}

/** Minimap color inputs: status and errors. A summary edit does not change this. */
export function canvasAlertKey(
  nodes: readonly { id: string; status: string }[],
  issues: readonly { nodeId: string; severity: string }[],
): string {
  let key = "";
  for (const node of nodes) key += `${node.id}:${node.status},`;
  key += "|";
  for (const issue of issues) {
    if (issue.severity === "error") key += `${issue.nodeId},`;
  }
  return key;
}

export function positionSignature(
  nodes: readonly { id: string; x: number | null; y: number | null }[],
): string {
  let key = "";
  for (const node of nodes) {
    key += `${node.id}:${node.x ?? ""},${node.y ?? ""};`;
  }
  return key;
}

export function positionStamp(x: number, y: number): string {
  return `${x}:${y}`;
}

/**
 * Summary and body as the card shows them.
 * A geometry rewrite trims the body, so that trailing whitespace is not a content edit.
 */
export function knowledgeContentSignature(summary: string, value: string): string {
  return `${summary.trim()}\0${value.replace(/\s+$/, "")}`;
}

/** Replace the named records. Every other element keeps its reference. */
export function patchById<T extends { id: string }>(
  items: readonly T[],
  ids: ReadonlySet<string>,
  recipe: (item: T) => T,
): readonly T[] {
  if (ids.size === 0) return items;
  let changed = false;
  const next = items.map((item) => {
    if (!ids.has(item.id)) return item;
    const patched = recipe(item);
    if (patched !== item) changed = true;
    return patched;
  });
  return changed ? next : items;
}

export function replaceOneRecord<T extends { id: string }>(
  records: readonly T[],
  index: ReadonlyMap<string, T>,
  next: T,
): { records: T[]; index: Map<string, T> } {
  const listed = records.map((record) => (record.id === next.id ? next : record));
  const indexed = new Map(index);
  indexed.set(next.id, next);
  return { records: listed, index: indexed };
}

/** Focus card plus citation-linked neighbours (same rule as canvas dimming used to use). */
export function focusSpotlightIds(
  focusedId: string | null,
  nodes: readonly { id: string; key: string; relations: readonly string[] }[],
): ReadonlySet<string> | null {
  if (focusedId == null) return null;
  const focus = nodes.find((n) => n.id === focusedId);
  if (!focus) return new Set([focusedId]);
  const focusKey = normalizeKey(focus.key);
  const ids = new Set<string>([focusedId]);
  for (const node of nodes) {
    if (node.id === focusedId) continue;
    const ownKey = normalizeKey(node.key);
    if (!ownKey) continue;
    if (focus.relations.includes(ownKey) || node.relations.includes(focusKey)) {
      ids.add(node.id);
    }
  }
  return ids;
}

export function edgeOpacity(
  edge: { source: string; target: string },
  spotlight: ReadonlySet<string> | null,
): number {
  if (spotlight == null) return 1;
  if (spotlight.has(edge.source) && spotlight.has(edge.target)) return 1;
  return 0.16;
}

/** Ease-out cubic. Zero at the start, one at the end, fast then settling. */
export function easeOutCubic(t: number): number {
  const clamped = t < 0 ? 0 : t > 1 ? 1 : t;
  return 1 - (1 - clamped) ** 3;
}

export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * Keep items that disappeared so a leave transition can finish.
 * A later snapshot that still misses them does not mark them again.
 * An id that comes back is the live item; `cancelled` is the dropped ghost.
 */
export function retainLeaving<T extends { id: string }>(
  previous: readonly T[],
  next: readonly T[],
  isLeaving: (item: T) => boolean,
  markLeaving: (item: T) => T,
): { items: T[]; started: string[]; cancelled: string[] } {
  const nextIds = new Set(next.map((item) => item.id));
  const started: string[] = [];
  const cancelled: string[] = [];
  const ghosts: T[] = [];
  for (const item of previous) {
    if (nextIds.has(item.id)) {
      if (isLeaving(item)) cancelled.push(item.id);
      continue;
    }
    if (isLeaving(item)) {
      ghosts.push(item);
      continue;
    }
    ghosts.push(markLeaving(item));
    started.push(item.id);
  }
  return {
    items: ghosts.length === 0 ? (next as T[]) : [...next, ...ghosts],
    started,
    cancelled,
  };
}

interface FocusEdge {
  source: string;
  target: string;
  data?: { opacity?: number; leaving?: boolean };
  zIndex?: number;
}

/** Dim edges outside the spotlight; refresh z-index when focus moves. */
export function patchEdgesForFocus<T extends FocusEdge>(
  edges: readonly T[],
  prevFocus: string | null,
  nextFocus: string | null,
  nodes: readonly { id: string; key: string; relations: readonly string[] }[],
  zIndexFor: (
    edge: { source: string; target: string },
    spotlight: ReadonlySet<string> | null,
  ) => number,
): readonly T[] {
  if (prevFocus === nextFocus) return edges;
  const spotlight = focusSpotlightIds(nextFocus, nodes);
  let changed = false;
  const next = edges.map((edge) => {
    if (edge.data?.leaving) return edge;
    const opacity = edgeOpacity(edge, spotlight);
    const zIndex = zIndexFor(edge, spotlight);
    if ((edge.data?.opacity ?? 1) === opacity && edge.zIndex === zIndex) return edge;
    changed = true;
    const data = edge.data ? { ...edge.data, opacity } : { opacity };
    return { ...edge, zIndex, data };
  });
  return changed ? next : edges;
}

interface ReconciledEdge {
  id: string;
  source: string;
  target: string;
  data?: { variant?: string; stroke?: string; opacity?: number };
}

/**
 * Swap in edges whose endpoints or variant changed.
 * An edge that still describes the same citation keeps its object.
 */
export function reconcileEdges<T extends ReconciledEdge>(
  prev: readonly T[],
  laid: readonly LaidOutEdge[],
  spotlight: ReadonlySet<string> | null,
  stroke: string,
  create: (edge: LaidOutEdge, opacity: number, stroke: string, zIndex: number) => T,
  zIndexFor: (
    edge: { source: string; target: string },
    spotlight: ReadonlySet<string> | null,
  ) => number,
): readonly T[] {
  const prevById = new Map(prev.map((edge) => [edge.id, edge]));
  const next = laid.map((edge) => {
    const opacity = edgeOpacity(edge, spotlight);
    const zIndex = zIndexFor(edge, spotlight);
    const old = prevById.get(edge.id);
    const oldZ = (old as { zIndex?: number } | undefined)?.zIndex ?? zIndex;
    if (
      old &&
      old.source === edge.source &&
      old.target === edge.target &&
      old.data?.variant === edge.variant &&
      old.data?.stroke === stroke &&
      old.data?.opacity === opacity &&
      oldZ === zIndex
    ) {
      return old;
    }
    return create(edge, opacity, stroke, zIndex);
  });
  if (next.length === prev.length) {
    let same = true;
    for (let i = 0; i < next.length; i++) {
      if (next[i] !== prev[i]) {
        same = false;
        break;
      }
    }
    if (same) return prev;
  }
  return next;
}

function sameIssueList(left: readonly KnowledgeIssue[], right: readonly KnowledgeIssue[]): boolean {
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i++) {
    const a = left[i];
    const b = right[i];
    if (
      a.nodeId !== b.nodeId ||
      a.severity !== b.severity ||
      a.code !== b.code ||
      a.message !== b.message ||
      (a.ref ?? "") !== (b.ref ?? "")
    ) {
      return false;
    }
  }
  return true;
}

/** Keep per-node issue arrays whose contents did not change. */
export function reuseIssueGroups(
  previous: ReadonlyMap<string, KnowledgeIssue[]>,
  next: ReadonlyMap<string, KnowledgeIssue[]>,
): Map<string, KnowledgeIssue[]> {
  let same = previous.size === next.size;
  const reused = new Map<string, KnowledgeIssue[]>();
  for (const [id, list] of next) {
    const prior = previous.get(id);
    if (prior && sameIssueList(prior, list)) reused.set(id, prior);
    else {
      same = false;
      reused.set(id, list);
    }
  }
  if (same) return previous as Map<string, KnowledgeIssue[]>;
  return reused;
}
