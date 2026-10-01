import { Folder } from "@phosphor-icons/react";
import { memo, useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import {
  Background,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  ViewportPortal,
  applyNodeChanges,
  useReactFlow,
  useNodesState,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
} from "@xyflow/react";

import { createCanvasMotion, isCanvasGesture, zoomChanged } from "../../lib/knowledge/canvasMotion";
import { fitKnowledgeFolders } from "../../lib/knowledge/fitFolders";
import {
  canvasAlertKey,
  easeOutCubic,
  edgeOpacity,
  focusSpotlightIds,
  knowledgeStructureKey,
  patchById,
  patchEdgesForFocus,
  positionSignature,
  positionStamp,
  prefersReducedMotion,
  reconcileEdges,
  relationSignature,
  retainLeaving,
} from "../../lib/knowledge/flowProjection";
import {
  KNOWLEDGE_GRID,
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_WIDTH,
  knowledgeRelationEdges,
  layoutKnowledgeGraph,
  type LaidOutEdge,
} from "../../lib/knowledge/layoutGraph";
import { nodeHasError } from "../../lib/knowledge/validate";
import {
  worldFromFlowNode,
  worldToFlow,
  worldWritesForPositionChanges,
  type FlowPointNode,
  type WorldPoint,
} from "../../lib/knowledge/world";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeFlowCard } from "./KnowledgeFlowCard";
import { KnowledgeRelationEdge, type KnowledgeRelationEdgeData } from "./KnowledgeRelationEdge";

type KnowledgeNodeData = { nodeId: string; arrive?: boolean };
type KnowledgeFolderData = { folderId: string };
type KnowledgeFlowNodeType =
  | Node<KnowledgeNodeData, "knowledge">
  | Node<KnowledgeFolderData, "knowledgeFolder">;

/** Canvas grid: the Background dot pitch, and the step dragged nodes snap to. */
const GRID_SIZE = KNOWLEDGE_GRID;

/** How long a removed card stays mounted so the shrink-fade can finish. */
const CARD_EXIT_MS = 260;
/** Edge opacity fade; slightly shorter than the card so the line is gone first. */
const EDGE_EXIT_MS = 220;
/** Ease-out move. Edges follow because the position state itself is interpolated. */
const POSITION_EASE_MS = 280;

/** Expand/collapse only — manual resize must not inherit this transition. */
const CARD_CHROME_MS = 240;
const CARD_SIZE_TRANSITION = `width ${CARD_CHROME_MS}ms cubic-bezier(0.4, 0, 0.2, 1), height ${CARD_CHROME_MS}ms cubic-bezier(0.4, 0, 0.2, 1)`;

interface GraphColors {
  muted: string;
  amber: string;
  red: string;
  accent: string;
  line: string;
  panel: string;
  mask: string;
}

function readToken(name: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  return (
    getComputedStyle(document.documentElement).getPropertyValue(name).trim() ||
    fallback
  );
}

function readColors(): GraphColors {
  return {
    muted: readToken("--_dk-text-muted", "#777"),
    amber: readToken("--_dk-amber-500", "#f59e0b"),
    red: readToken("--_dk-red-500", "#ef4444"),
    accent: readToken("--_dk-accent-ring", "#bacbcd"),
    line: readToken("--_dk-line-visible", "#333"),
    panel: readToken("--_dk-editor", "#1a1a1a"),
    mask:
      document.documentElement.getAttribute("data-dv-theme") === "light"
        ? "rgba(0, 0, 0, 0.12)"
        : "rgba(255, 255, 255, 0.08)",
  };
}

function useGraphColors(): GraphColors {
  const [colors, setColors] = useState(readColors);
  useEffect(() => {
    const sync = () => setColors(readColors());
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-dv-theme", "class", "style"],
    });
    return () => observer.disconnect();
  }, []);
  return colors;
}

function cssDimension(
  value: CSSProperties["width"],
  fallback: number,
): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number.parseFloat(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function cardStyle(
  open: boolean,
  width: number | null | undefined,
  height: number | null | undefined,
): CSSProperties {
  if (!open) {
    return { width: KNOWLEDGE_NODE_WIDTH, height: KNOWLEDGE_NODE_HEIGHT };
  }
  return {
    width: width ?? KNOWLEDGE_NODE_WIDTH,
    height: height ?? undefined,
  };
}

/** React Flow reads `width` / `height` / `measured`, not only `style`. */
export function knowledgeCardFlowLayout(
  open: boolean,
  saved: { w: number | null; h: number | null } | undefined,
): { style: CSSProperties; width: number; height: number | undefined } {
  const style = cardStyle(open, saved?.w, saved?.h);
  if (!open) {
    return { style, width: KNOWLEDGE_NODE_WIDTH, height: KNOWLEDGE_NODE_HEIGHT };
  }
  return {
    style,
    width: saved?.w ?? KNOWLEDGE_NODE_WIDTH,
    height: saved?.h ?? undefined,
  };
}

function flowNodeSize(node: Node | undefined): { w: number; h: number } {
  const w =
    node?.measured?.width ??
    node?.width ??
    cssDimension(node?.style?.width, KNOWLEDGE_NODE_WIDTH);
  const h =
    node?.measured?.height ??
    node?.height ??
    cssDimension(node?.style?.height, KNOWLEDGE_NODE_HEIGHT);
  return { w, h };
}

const KnowledgeFolderNode = memo(function KnowledgeFolderNode({
  data,
}: NodeProps<Node<KnowledgeFolderData, "knowledgeFolder">>) {
  const folder = useKnowledgeStore((s) => s.folderById.get(data.folderId));
  if (!folder) return null;
  return (
    <div className="knowledge-flow-folder">
      <div className="knowledge-flow-folder-label">
        <Folder size={14} weight="fill" aria-hidden />
        <span className="truncate">{folder.name}</span>
      </div>
    </div>
  );
});

const nodeTypes = {
  knowledge: KnowledgeFlowCard,
  knowledgeFolder: KnowledgeFolderNode,
};
const edgeTypes = { knowledgeRelation: KnowledgeRelationEdge };

// Manual z-index ladder (bottom → top): canvas bg, folder frames, edges, cards.
export const KNOWLEDGE_FOLDER_Z = 0;
export const KNOWLEDGE_EDGE_Z = 5;
/** Citation edges inside the focus spotlight sit above the scrim. */
export const KNOWLEDGE_EDGE_ABOVE_SCRIM_Z = 36;
/** Full-canvas focus scrim in ViewportPortal; focused cards must sit above this. */
export const KNOWLEDGE_FOCUS_SCRIM_Z = 35;
const CARD_Z = 10;
const CARD_Z_EXPANDED = 30;
const CARD_Z_LINKED = 38;
const CARD_Z_LINKED_EXPANDED = 40;
const CARD_Z_FOCUSED = 42;
const CARD_Z_FOCUSED_EXPANDED = 45;
const CARD_Z_DRAGGING = 50;

export function knowledgeEdgeZIndex(
  edge: { source: string; target: string },
  spotlight: ReadonlySet<string> | null,
): number {
  if (spotlight == null) return KNOWLEDGE_EDGE_Z;
  if (spotlight.has(edge.source) && spotlight.has(edge.target)) {
    return KNOWLEDGE_EDGE_ABOVE_SCRIM_Z;
  }
  return KNOWLEDGE_EDGE_Z;
}

/** Stacking rank of one canvas card. An expanded card grows over its
 *  neighbours, so it rises above every collapsed card; the focused expanded
 *  card sits on top of all of them, and a dragged card outranks everything
 *  while the gesture lasts. */
export function knowledgeCardZIndex(
  open: boolean,
  focused: boolean,
  linked: boolean,
  dragging: boolean,
): number {
  if (dragging) return CARD_Z_DRAGGING;
  if (focused) return open ? CARD_Z_FOCUSED_EXPANDED : CARD_Z_FOCUSED;
  if (linked) return open ? CARD_Z_LINKED_EXPANDED : CARD_Z_LINKED;
  if (open) return CARD_Z_EXPANDED;
  return CARD_Z;
}

function canvasLinked(
  nodeId: string,
  focusedId: string | null,
  spotlight: ReadonlySet<string> | null,
): boolean {
  return spotlight != null && nodeId !== focusedId && spotlight.has(nodeId);
}

function flowPosition(
  world: WorldPoint,
  parentId: string | null,
  folderWorld: ReadonlyMap<string, WorldPoint>,
): WorldPoint {
  return worldToFlow(world, parentId ? (folderWorld.get(parentId) ?? null) : null);
}

function buildFlowNode(
  node: ReturnType<typeof layoutKnowledgeGraph>["nodes"][number],
  focusedId: string | null,
  spotlight: ReadonlySet<string> | null,
  draggingId: string | null,
  prev: KnowledgeFlowNodeType | undefined,
  open: boolean,
  saved: { w: number | null; h: number | null } | undefined,
  position: WorldPoint,
  arrive: boolean,
): KnowledgeFlowNodeType {
  const focused = node.nodeId === focusedId;
  const linked = spotlight != null && !focused && spotlight.has(node.nodeId);
  const zIndex = knowledgeCardZIndex(
    open,
    focused,
    linked,
    node.nodeId === draggingId,
  );
  const layout = knowledgeCardFlowLayout(open, saved);
  return {
    id: node.id,
    type: "knowledge",
    position: prev?.position ?? position,
    parentId: node.parentId ?? undefined,
    data: arrive ? { nodeId: node.nodeId, arrive: true } : { nodeId: node.nodeId },
    draggable: true,
    connectable: false,
    zIndex,
    style: layout.style,
    width: layout.width,
    height: layout.height,
  };
}

function buildFolderNode(
  folder: ReturnType<typeof layoutKnowledgeGraph>["folders"][number],
  prev: KnowledgeFlowNodeType | undefined,
  position: WorldPoint,
): KnowledgeFlowNodeType {
  const width = cssDimension(prev?.style?.width, folder.width);
  const height = cssDimension(prev?.style?.height, folder.height);
  return {
    id: folder.id,
    type: "knowledgeFolder",
    position: prev?.position ?? position,
    parentId: folder.parentId ?? undefined,
    data: { folderId: folder.folderId },
    dragHandle: ".knowledge-flow-folder-label",
    draggable: true,
    connectable: false,
    selectable: false,
    zIndex: KNOWLEDGE_FOLDER_Z,
    width,
    height,
    style: { width, height },
  };
}

function folderDepth(
  parentId: string | null,
  byId: Map<string, { parentId: string | null }>,
): number {
  let depth = 0;
  let current = parentId;
  const seen = new Set<string>();
  while (current && !seen.has(current)) {
    seen.add(current);
    depth += 1;
    current = byId.get(current)?.parentId ?? null;
  }
  return depth;
}

function changedIds(prev: ReadonlySet<string>, next: ReadonlySet<string>): Set<string> {
  const ids = new Set<string>();
  for (const id of next) if (!prev.has(id)) ids.add(id);
  for (const id of prev) if (!next.has(id)) ids.add(id);
  return ids;
}

function stripCardSizeTransition(node: KnowledgeFlowNodeType): KnowledgeFlowNodeType {
  if (node.type !== "knowledge" || !node.style?.transition) return node;
  const { transition: _drop, ...rest } = node.style;
  const style = Object.keys(rest).length > 0 ? rest : undefined;
  return { ...node, style };
}

function withCardChrome(
  node: KnowledgeFlowNodeType,
  chrome: {
    open: boolean;
    focused: boolean;
    linked: boolean;
    dragging: boolean;
    saved: { w: number | null; h: number | null } | undefined;
    updateStyle: boolean;
    animateSize?: boolean;
  },
): KnowledgeFlowNodeType {
  if (node.type !== "knowledge") return node;
  const zIndex = knowledgeCardZIndex(
    chrome.open,
    chrome.focused,
    chrome.linked,
    chrome.dragging,
  );
  if (!chrome.updateStyle) {
    if (node.zIndex === zIndex) return node;
    return { ...node, zIndex };
  }
  const layout = knowledgeCardFlowLayout(chrome.open, chrome.saved);
  const flowStyle = chrome.animateSize
    ? { ...layout.style, transition: CARD_SIZE_TRANSITION }
    : layout.style;
  const unchanged =
    node.zIndex === zIndex &&
    node.style?.width === flowStyle.width &&
    node.style?.height === flowStyle.height &&
    node.style?.transition === flowStyle.transition &&
    node.width === layout.width &&
    node.height === layout.height &&
    node.measured == null;
  if (unchanged) return node;
  return {
    ...node,
    zIndex,
    style: flowStyle,
    width: layout.width,
    height: layout.height,
    measured: undefined,
  };
}

function toFlowEdge(
  edge: LaidOutEdge,
  opacity: number,
  stroke: string,
  zIndex: number,
  draw: boolean,
): Edge {
  return {
    id: edge.id,
    source: edge.source,
    target: edge.target,
    type: "knowledgeRelation",
    zIndex,
    data: {
      variant: edge.variant,
      stroke,
      opacity,
      draw,
    },
    markerEnd: {
      type: MarkerType.ArrowClosed,
      width: 11,
      height: 11,
      color: stroke,
    },
  };
}

function toFlowEdges(
  edges: LaidOutEdge[],
  stroke: string,
  spotlight: ReadonlySet<string> | null,
  draw: boolean,
): Edge[] {
  return edges.map((edge) =>
    toFlowEdge(
      edge,
      edgeOpacity(edge, spotlight),
      stroke,
      knowledgeEdgeZIndex(edge, spotlight),
      draw,
    ),
  );
}

function isCardLeaving(node: { className?: string }): boolean {
  return node.className?.split(/\s+/).includes("knowledge-node-leave") ?? false;
}

function markCardLeaving(node: KnowledgeFlowNodeType): KnowledgeFlowNodeType {
  return {
    ...node,
    className: "knowledge-node-leave",
    draggable: false,
    selectable: false,
  };
}

function isEdgeLeaving(edge: { data?: unknown }): boolean {
  return (edge.data as { leaving?: boolean } | undefined)?.leaving === true;
}

function markEdgeLeaving(edge: Edge): Edge {
  const data = (edge.data ?? {}) as KnowledgeRelationEdgeData;
  return {
    ...edge,
    selectable: false,
    data: { ...data, opacity: 0, leaving: true },
  };
}

function KnowledgeFocusScrim() {
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  if (focusedId == null) return null;
  return (
    <ViewportPortal>
      <div className="knowledge-focus-scrim" aria-hidden />
    </ViewportPortal>
  );
}

function FocusViewport() {
  const { setCenter, getNode } = useReactFlow();
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const nonce = useKnowledgeStore((s) => s.focusNonce);
  const lastPannedNonce = useRef(0);
  useEffect(() => {
    if (nonce === lastPannedNonce.current) return;
    lastPannedNonce.current = nonce;
    if (focusedId == null) return;
    const flowNode = getNode(String(focusedId));
    if (!flowNode) return;
    const { w, h } = flowNodeSize(flowNode);
    let x = flowNode.position.x;
    let y = flowNode.position.y;
    let parentId = flowNode.parentId;
    const seen = new Set<string>();
    while (parentId && !seen.has(parentId)) {
      seen.add(parentId);
      const parent = getNode(parentId);
      if (!parent) break;
      x += parent.position.x;
      y += parent.position.y;
      parentId = parent.parentId;
    }
    const timer = window.setTimeout(() => {
      void setCenter(
        x + w / 2,
        y + h / 2,
        { zoom: 1, duration: nonce === 0 ? 0 : 280 },
      );
    }, 40);
    return () => window.clearTimeout(timer);
  }, [focusedId, nonce, getNode, setCenter]);
  return null;
}

export function KnowledgeGraph() {
  const structureKey = useKnowledgeStore((s) => knowledgeStructureKey(s.nodes, s.folders));
  const structureNonce = useKnowledgeStore((s) => s.structureNonce);
  const relationKey = useKnowledgeStore((s) => relationSignature(s.nodes));
  const positionKey = useKnowledgeStore((s) => positionSignature(s.nodes));
  const graphExpanded = useKnowledgeStore((s) => s.graphExpanded);
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const alertKey = useKnowledgeStore((s) => canvasAlertKey(s.nodes, s.issues));
  const selectFromGraph = useKnowledgeStore((s) => s.selectFromGraph);
  const clearCanvasFocus = useKnowledgeStore((s) => s.clearCanvasFocus);
  const colors = useGraphColors();
  const colorsRef = useRef(colors);
  colorsRef.current = colors;
  const [flowNodes, setFlowNodes] = useNodesState<KnowledgeFlowNodeType>([]);
  const [flowEdges, setFlowEdges] = useState<Edge[]>([]);
  const flowRef = useRef(flowNodes);
  const draggingIdRef = useRef<string | null>(null);
  const geometryEcho = useRef(new Map<string, string>());
  const positionHandled = useRef<string | null>(null);
  const repaired = useRef(new Set<string>());
  const seenStructure = useRef(structureNonce);
  const expandedSeen = useRef<Set<string> | null>(null);
  const focusSeen = useRef<string | null>(null);
  const relationSeen = useRef<string | null>(null);
  const edgesMayDraw = useRef(false);
  const mountedRef = useRef(true);
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasMotion = useRef(createCanvasMotion());
  const zoomSeen = useRef<number | null>(null);
  const exitTimers = useRef(new Map<string, number>());
  const movesRef = useRef(
    new Map<
      string,
      { fromX: number; fromY: number; toX: number; toY: number; started: number }
    >(),
  );
  const moveFrame = useRef<number | null>(null);

  function cancelExit(key: string) {
    const timer = exitTimers.current.get(key);
    if (timer == null) return;
    window.clearTimeout(timer);
    exitTimers.current.delete(key);
  }

  function armExit(key: string, delay: number, run: () => void) {
    if (exitTimers.current.has(key)) return;
    const timer = window.setTimeout(() => {
      exitTimers.current.delete(key);
      if (!mountedRef.current) return;
      run();
    }, delay);
    exitTimers.current.set(key, timer);
  }

  function dropLeavingCard(id: string) {
    setFlowNodes((current) => {
      const next = current.filter((node) => node.id !== id || !isCardLeaving(node));
      if (next.length === current.length) return current;
      const fitted = fitKnowledgeFolders(next);
      flowRef.current = fitted;
      return fitted;
    });
  }

  function dropLeavingEdge(id: string) {
    setFlowEdges((current) => {
      const next = current.filter((edge) => edge.id !== id || !isEdgeLeaving(edge));
      return next.length === current.length ? current : next;
    });
  }

  function tickMoves() {
    const moves = movesRef.current;
    const dragging = draggingIdRef.current;
    if (dragging) moves.delete(dragging);
    const now = performance.now();
    const frame = new Map<string, { x: number; y: number }>();
    const done: string[] = [];
    let pending = false;
    for (const [id, move] of moves) {
      const eased = easeOutCubic((now - move.started) / POSITION_EASE_MS);
      frame.set(id, {
        x: move.fromX + (move.toX - move.fromX) * eased,
        y: move.fromY + (move.toY - move.fromY) * eased,
      });
      if (eased < 1) pending = true;
      else done.push(id);
    }
    for (const id of done) moves.delete(id);
    if (frame.size > 0) {
      setFlowNodes((current) => {
        let changed = false;
        const next = current.map((node) => {
          const point = frame.get(node.id);
          if (!point) return node;
          if (node.position.x === point.x && node.position.y === point.y) return node;
          changed = true;
          return { ...node, position: point };
        });
        if (!changed) return current;
        flowRef.current = next;
        return next;
      });
    }
    if (!mountedRef.current) return;
    if (pending) {
      moveFrame.current = requestAnimationFrame(tickMoves);
      return;
    }
    moveFrame.current = null;
    setFlowNodes((current) => {
      const fitted = fitKnowledgeFolders(current);
      if (fitted === current) return current;
      flowRef.current = fitted;
      return fitted;
    });
  }

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      canvasMotion.current.dispose();
      if (moveFrame.current != null) cancelAnimationFrame(moveFrame.current);
      for (const timer of exitTimers.current.values()) window.clearTimeout(timer);
      exitTimers.current.clear();
    };
  }, []);

  const commitNodes = useCallback(
    (next: KnowledgeFlowNodeType[]) => {
      if (next === flowRef.current) return;
      flowRef.current = next;
      setFlowNodes(next);
    },
    [setFlowNodes],
  );

  const onNodesChange = useCallback(
    (changes: NodeChange<KnowledgeFlowNodeType>[]) => {
      const next = fitKnowledgeFolders(applyNodeChanges(changes, flowRef.current));
      commitNodes(next);
      const state = useKnowledgeStore.getState();
      for (const write of worldWritesForPositionChanges(changes, next)) {
        geometryEcho.current.set(write.id, positionStamp(write.x, write.y));
        void state.saveGeometry(write.id, { x: write.x, y: write.y });
      }
      for (const change of changes) {
        if (
          change.type === "dimensions" &&
          change.resizing === false &&
          change.dimensions &&
          state.graphExpanded.has(change.id)
        ) {
          void state.saveGeometry(change.id, {
            w: change.dimensions.width,
            h: change.dimensions.height,
          });
        }
      }
    },
    [commitNodes],
  );

  useEffect(() => {
    const state = useKnowledgeStore.getState();
    const fresh = structureNonce !== seenStructure.current;
    seenStructure.current = structureNonce;
    relationSeen.current = relationSignature(state.nodes);
    const laid = layoutKnowledgeGraph(state.nodes, state.issues, state.folders);
    for (const node of laid.nodes) {
      const source = state.byId.get(node.nodeId);
      if (!source || (source.x != null && source.y != null)) continue;
      const stamp = positionStamp(node.x, node.y);
      const token = `${node.nodeId}:${stamp}`;
      if (repaired.current.has(token)) continue;
      repaired.current.add(token);
      geometryEcho.current.set(node.nodeId, stamp);
      void state.saveGeometry(node.nodeId, { x: node.x, y: node.y }).then((ok) => {
        if (!ok) repaired.current.delete(token);
      });
    }
    const draggingId = draggingIdRef.current;
    const drawEdges = edgesMayDraw.current;
    edgesMayDraw.current = true;
    setFlowNodes((current) => {
      const previousById = new Map(current.map((node) => [node.id, node]));
      const prevById = fresh
        ? new Map<string, KnowledgeFlowNodeType>()
        : previousById;
      const folderByFlowId = new Map(laid.folders.map((folder) => [folder.id, folder]));
      const folderWorld = new Map(
        laid.folders.map((folder) => [folder.id, { x: folder.x, y: folder.y }]),
      );
      const folderNodes = [...laid.folders]
        .sort(
          (a, b) =>
            folderDepth(a.parentId, folderByFlowId) - folderDepth(b.parentId, folderByFlowId),
        )
        .map((folder) => {
          const prev = prevById.get(folder.id);
          if (prev?.type === "knowledgeFolder") return prev;
          return buildFolderNode(
            folder,
            prev,
            flowPosition({ x: folder.x, y: folder.y }, folder.parentId, folderWorld),
          );
        });
      const spotlight = focusSpotlightIds(state.focusedId, state.nodes);
      const liveIds = new Set<string>();
      for (const node of current) {
        if (node.type === "knowledge" && !isCardLeaving(node)) liveIds.add(node.id);
      }
      const cardNodes = laid.nodes.map((node) => {
        const prev = previousById.get(node.id);
        const source = state.byId.get(node.nodeId);
        return buildFlowNode(
          node,
          state.focusedId,
          spotlight,
          draggingId,
          prev?.type === "knowledge" ? prev : undefined,
          state.graphExpanded.has(node.nodeId),
          source,
          flowPosition({ x: node.x, y: node.y }, node.parentId, folderWorld),
          liveIds.size > 0 && !liveIds.has(node.id),
        );
      });
      const folderIds = new Set(folderNodes.map((folder) => folder.id));
      const presence = prefersReducedMotion()
        ? { items: cardNodes, started: [] as string[], cancelled: [] as string[] }
        : retainLeaving(
            current.filter((node) => node.type === "knowledge"),
            cardNodes,
            isCardLeaving,
            markCardLeaving,
          );
      const cards = presence.items.filter(
        (node) => !isCardLeaving(node) || !node.parentId || folderIds.has(node.parentId),
      );
      if (presence.started.length > 0 || presence.cancelled.length > 0) {
        const started = presence.started.filter((id) => cards.some((node) => node.id === id));
        const cancelled = presence.cancelled;
        queueMicrotask(() => {
          if (!mountedRef.current) return;
          for (const id of cancelled) cancelExit(`card:${id}`);
          for (const id of started) {
            armExit(`card:${id}`, CARD_EXIT_MS, () => dropLeavingCard(id));
          }
        });
      }
      const next = fitKnowledgeFolders([...folderNodes, ...cards]);
      flowRef.current = next;
      return next;
    });
    setFlowEdges((existing) => {
      const laidEdges = toFlowEdges(
        laid.edges,
        colorsRef.current.muted,
        focusSpotlightIds(state.focusedId, state.nodes),
        drawEdges,
      );
      if (prefersReducedMotion()) return laidEdges;
      const retained = retainLeaving(existing, laidEdges, isEdgeLeaving, markEdgeLeaving);
      if (retained.started.length > 0 || retained.cancelled.length > 0) {
        const started = retained.started;
        const cancelled = retained.cancelled;
        queueMicrotask(() => {
          if (!mountedRef.current) return;
          for (const id of cancelled) cancelExit(`edge:${id}`);
          for (const id of started) {
            armExit(`edge:${id}`, EDGE_EXIT_MS, () => dropLeavingEdge(id));
          }
        });
      }
      return retained.items;
    });
  }, [structureKey, structureNonce, setFlowNodes]);

  useEffect(() => {
    if (positionHandled.current === positionKey) return;
    positionHandled.current = positionKey;
    const state = useKnowledgeStore.getState();
    const echoed = geometryEcho.current;
    geometryEcho.current = new Map();
    const reduce = prefersReducedMotion();
    setFlowNodes((current) => {
      if (current.length === 0) return current;
      const lookup = new Map<string, FlowPointNode>();
      for (const node of current) {
        lookup.set(node.id, {
          id: node.id,
          type: node.type,
          parentId: node.parentId,
          position: node.position,
        });
      }
      let changed = false;
      const next = current.map((node) => {
        if (node.type !== "knowledge" || isCardLeaving(node)) return node;
        const source = state.byId.get(node.id);
        if (!source || source.x == null || source.y == null) return node;
        const stamp = positionStamp(source.x, source.y);
        if (echoed.get(node.id) === stamp) return node;
        const parent = node.parentId ? lookup.get(node.parentId) : undefined;
        const parentWorld = parent ? worldFromFlowNode(parent, lookup) : null;
        const position = worldToFlow({ x: source.x, y: source.y }, parentWorld);
        const dx = position.x - node.position.x;
        const dy = position.y - node.position.y;
        if (dx * dx + dy * dy < 0.25) return node;
        changed = true;
        return { ...node, position };
      });
      if (!changed) return current;
      if (reduce) {
        movesRef.current.clear();
        const fitted = fitKnowledgeFolders(next);
        flowRef.current = fitted;
        return fitted;
      }
      const now = performance.now();
      const moves = movesRef.current;
      const nextById = new Map(next.map((node) => [node.id, node]));
      for (const node of current) {
        const target = nextById.get(node.id);
        if (
          !target ||
          (target.position.x === node.position.x && target.position.y === node.position.y)
        ) {
          continue;
        }
        moves.set(node.id, {
          fromX: node.position.x,
          fromY: node.position.y,
          toX: target.position.x,
          toY: target.position.y,
          started: now,
        });
      }
      queueMicrotask(() => {
        if (!mountedRef.current || movesRef.current.size === 0) return;
        if (moveFrame.current != null) return;
        moveFrame.current = requestAnimationFrame(tickMoves);
      });
      return current;
    });
  }, [positionKey, setFlowNodes]);

  useEffect(() => {
    const prev = expandedSeen.current;
    expandedSeen.current = graphExpanded;
    if (prev == null) return;
    const ids = changedIds(prev, graphExpanded);
    if (ids.size === 0) return;
    const state = useKnowledgeStore.getState();
    const draggingId = draggingIdRef.current;
    const spotlight = focusSpotlightIds(state.focusedId, state.nodes);
    const animateSize =
      typeof window !== "undefined" &&
      !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    setFlowNodes((current) => {
      if (current.length === 0) return current;
      const next = patchById(current, ids, (node) => {
        if (isCardLeaving(node)) return node;
        return withCardChrome(node, {
          open: graphExpanded.has(node.id),
          focused: node.id === state.focusedId,
          linked: canvasLinked(node.id, state.focusedId, spotlight),
          dragging: node.id === draggingId,
          saved: state.byId.get(node.id),
          updateStyle: true,
          animateSize,
        });
      }) as KnowledgeFlowNodeType[];
      if (next === current) return current;
      flowRef.current = next;
      return next;
    });
    const timer = window.setTimeout(() => {
      setFlowNodes((current) => {
        if (current.length === 0) return current;
        const cleared = patchById(current, ids, stripCardSizeTransition) as KnowledgeFlowNodeType[];
        if (cleared === current) return current;
        flowRef.current = cleared;
        return cleared;
      });
    }, CARD_CHROME_MS + 40);
    return () => window.clearTimeout(timer);
  }, [graphExpanded, setFlowNodes]);

  useEffect(() => {
    const prev = focusSeen.current;
    if (prev === focusedId) return;
    focusSeen.current = focusedId;
    const state = useKnowledgeStore.getState();
    const draggingId = draggingIdRef.current;
    const spotlight = focusSpotlightIds(focusedId, state.nodes);
    setFlowNodes((current) => {
      if (current.length === 0) return current;
      let changed = false;
      const next = current.map((node) => {
        if (node.type !== "knowledge") return node;
        const patched = withCardChrome(node, {
          open: state.graphExpanded.has(node.id),
          focused: node.id === focusedId,
          linked: canvasLinked(node.id, focusedId, spotlight),
          dragging: node.id === draggingId,
          saved: undefined,
          updateStyle: false,
        });
        if (patched !== node) changed = true;
        return patched;
      });
      if (!changed) return current;
      flowRef.current = next as KnowledgeFlowNodeType[];
      return next;
    });
    setFlowEdges((current) => {
      const patched = patchEdgesForFocus(
        current,
        prev,
        focusedId,
        state.nodes,
        knowledgeEdgeZIndex,
      );
      return patched === current ? current : (patched as Edge[]);
    });
  }, [focusedId, setFlowNodes]);

  useEffect(() => {
    if (relationSeen.current === relationKey) return;
    const previous = relationSeen.current;
    relationSeen.current = relationKey;
    if (previous == null) return;
    const state = useKnowledgeStore.getState();
    const laid = knowledgeRelationEdges(state.nodes, state.issues);
    const stroke = colorsRef.current.muted;
    const spotlight = focusSpotlightIds(state.focusedId, state.nodes);
    if (state.focusedId != null) {
      const draggingId = draggingIdRef.current;
      setFlowNodes((current) => {
        if (current.length === 0) return current;
        let changed = false;
        const next = current.map((node) => {
          if (node.type !== "knowledge") return node;
          const patched = withCardChrome(node, {
            open: state.graphExpanded.has(node.id),
            focused: node.id === state.focusedId,
            linked: canvasLinked(node.id, state.focusedId, spotlight),
            dragging: node.id === draggingId,
            saved: undefined,
            updateStyle: false,
          });
          if (patched !== node) changed = true;
          return patched;
        });
        if (!changed) return current;
        flowRef.current = next as KnowledgeFlowNodeType[];
        return next;
      });
    }
    setFlowEdges((current) => {
      const reconciled = reconcileEdges(
        current,
        laid,
        spotlight,
        stroke,
        (edge, opacity, edgeStroke, zIndex) =>
          toFlowEdge(edge, opacity, edgeStroke, zIndex, edgesMayDraw.current),
        knowledgeEdgeZIndex,
      );
      if (prefersReducedMotion()) {
        return reconciled === current ? current : (reconciled as Edge[]);
      }
      const retained = retainLeaving(
        current,
        reconciled as Edge[],
        isEdgeLeaving,
        markEdgeLeaving,
      );
      if (retained.started.length > 0 || retained.cancelled.length > 0) {
        const started = retained.started;
        const cancelled = retained.cancelled;
        queueMicrotask(() => {
          if (!mountedRef.current) return;
          for (const id of cancelled) cancelExit(`edge:${id}`);
          for (const id of started) {
            armExit(`edge:${id}`, EDGE_EXIT_MS, () => dropLeavingEdge(id));
          }
        });
      }
      return retained.items === current ? current : retained.items;
    });
  }, [relationKey]);

  useEffect(() => {
    const stroke = colors.muted;
    setFlowEdges((current) => {
      let changed = false;
      const next = current.map((edge) => {
        const data = edge.data as { stroke?: string; variant?: string; opacity?: number } | undefined;
        if (!data || data.stroke === stroke) return edge;
        changed = true;
        const markerEnd = edge.markerEnd;
        const marker =
          markerEnd && typeof markerEnd === "object"
            ? { ...markerEnd, color: stroke }
            : markerEnd;
        return { ...edge, data: { ...data, stroke }, markerEnd: marker };
      });
      return changed ? next : current;
    });
  }, [colors.muted]);

  function setDragging(id: string | null) {
    const prev = draggingIdRef.current;
    if (prev === id) return;
    draggingIdRef.current = id;
    if (id) movesRef.current.delete(id);
    const state = useKnowledgeStore.getState();
    const spotlight = focusSpotlightIds(state.focusedId, state.nodes);
    const ids = new Set<string>();
    if (prev) ids.add(prev);
    if (id) ids.add(id);
    const next = patchById(flowRef.current, ids, (node) =>
      withCardChrome(node, {
        open: state.graphExpanded.has(node.id),
        focused: node.id === state.focusedId,
        linked: canvasLinked(node.id, state.focusedId, spotlight),
        dragging: node.id === id,
        saved: undefined,
        updateStyle: false,
      }),
    );
    commitNodes(next as KnowledgeFlowNodeType[]);
  }

  return (
    <div ref={hostRef} className="relative h-full w-full">
      <ReactFlow
        nodes={flowNodes}
        onMove={(_, viewport) => {
          const previous = zoomSeen.current;
          zoomSeen.current = viewport.zoom;
          if (!zoomChanged(previous, viewport.zoom)) return;
          const host = hostRef.current;
          if (host) canvasMotion.current.note(host);
        }}
        edges={flowEdges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodeClick={(_, node) => {
          if (node.type !== "knowledge" || isCardLeaving(node)) return;
          selectFromGraph(node.id);
        }}
        onPaneClick={() => clearCanvasFocus()}
        onNodesChange={(changes) => {
          onNodesChange(changes);
          if (!changes.some(isCanvasGesture)) return;
          const host = hostRef.current;
          if (host) canvasMotion.current.note(host);
        }}
        onNodeDragStart={(_, node) => {
          if (node.type === "knowledge") setDragging(node.id);
        }}
        onNodeDragStop={() => setDragging(null)}
        nodesDraggable
        nodeDragThreshold={4}
        snapToGrid
        snapGrid={[GRID_SIZE, GRID_SIZE]}
        nodesConnectable={false}
        edgesReconnectable={false}
        elementsSelectable
        deleteKeyCode={null}
        minZoom={0.2}
        maxZoom={1.5}
        // Manual: the 'basic' default lets an edge inherit its endpoint's
        // z-index, so a focused card pushed its lines above every other card.
        zIndexMode="manual"
        proOptions={{ hideAttribution: false }}
      >
        <Background color={colors.line} gap={GRID_SIZE} size={1} />
        <Controls showInteractive={false} />
        <MiniMap
          pannable
          zoomable
          style={{ width: 140, height: 88 }}
          bgColor={colors.panel}
          maskColor={colors.mask}
          nodeColor={(node) => {
            void alertKey;
            const state = useKnowledgeStore.getState();
            const knowledge = state.byId.get(node.id);
            if (!knowledge) return colors.muted;
            if (nodeHasError(state.issues, knowledge.id)) return colors.red;
            if (knowledge.status === "pending") return colors.amber;
            if (knowledge.status === "disabled") return colors.muted;
            return colors.accent;
          }}
        />
        <FocusViewport />
        <KnowledgeFocusScrim />
      </ReactFlow>
    </div>
  );
}
