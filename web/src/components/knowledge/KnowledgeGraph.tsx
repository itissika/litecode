import { Folder } from "@phosphor-icons/react";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  Background,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  applyNodeChanges,
  useReactFlow,
  useNodesState,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
} from "@xyflow/react";

import { fitKnowledgeFolders } from "../../lib/knowledge/fitFolders";
import {
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_WIDTH,
  layoutKnowledgeGraph,
  type LaidOutEdge,
} from "../../lib/knowledge/layoutGraph";
import { nodeHasError } from "../../lib/knowledge/validate";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeFlowCard } from "./KnowledgeFlowCard";
import { KnowledgeRelationEdge } from "./KnowledgeRelationEdge";

type KnowledgeNodeData = { nodeId: string };
type KnowledgeFolderData = { folderId: string };
type KnowledgeFlowNodeType =
  | Node<KnowledgeNodeData, "knowledge">
  | Node<KnowledgeFolderData, "knowledgeFolder">;

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

function buildFlowNode(
  node: ReturnType<typeof layoutKnowledgeGraph>["nodes"][number],
  focusedId: string | null,
  prev: KnowledgeFlowNodeType | undefined,
  open: boolean,
  saved: { w: number | null; h: number | null } | undefined,
): KnowledgeFlowNodeType {
  return {
    id: node.id,
    type: "knowledge",
    position: prev?.position ?? { x: node.x, y: node.y },
    parentId: node.parentId ?? undefined,
    data: { nodeId: node.nodeId },
    draggable: true,
    connectable: false,
    zIndex: node.nodeId === focusedId ? 10 : 1,
    style: cardStyle(open, saved?.w, saved?.h),
  };
}

function buildFolderNode(
  folder: ReturnType<typeof layoutKnowledgeGraph>["folders"][number],
  prev: KnowledgeFlowNodeType | undefined,
): KnowledgeFlowNodeType {
  const width = cssDimension(prev?.style?.width, folder.width);
  const height = cssDimension(prev?.style?.height, folder.height);
  return {
    id: folder.id,
    type: "knowledgeFolder",
    position: prev?.position ?? { x: folder.x, y: folder.y },
    parentId: folder.parentId ?? undefined,
    data: { folderId: folder.folderId },
    dragHandle: ".knowledge-flow-folder-label",
    draggable: true,
    connectable: false,
    selectable: false,
    zIndex: 0,
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

function toFlowEdges(
  edges: LaidOutEdge[],
  colors: GraphColors,
  focusedId: string | null,
): Edge[] {
  return edges.map((edge) => {
    const hot =
      focusedId == null ||
      edge.source === String(focusedId) ||
      edge.target === String(focusedId);
    const stroke = colors.muted;
    return {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      type: "knowledgeRelation",
      data: {
        variant: edge.variant,
        stroke,
        opacity: hot ? 1 : 0.16,
      },
      markerEnd: {
        type: MarkerType.ArrowClosed,
        width: 16,
        height: 16,
        color: stroke,
      },
    };
  });
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
  const nodes = useKnowledgeStore((s) => s.nodes);
  const folders = useKnowledgeStore((s) => s.folders);
  const issues = useKnowledgeStore((s) => s.issues);
  const byId = useKnowledgeStore((s) => s.byId);
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const graphExpanded = useKnowledgeStore((s) => s.graphExpanded);
  const structureNonce = useKnowledgeStore((s) => s.structureNonce);
  const selectFromGraph = useKnowledgeStore((s) => s.selectFromGraph);
  const clearCanvasFocus = useKnowledgeStore((s) => s.clearCanvasFocus);
  const colors = useGraphColors();
  const laid = useMemo(
    () => layoutKnowledgeGraph(nodes, issues, folders),
    [nodes, issues, folders],
  );
  const [flowNodes, setFlowNodes] = useNodesState<KnowledgeFlowNodeType>([]);
  const onNodesChange = useCallback(
    (changes: NodeChange<KnowledgeFlowNodeType>[]) => {
      setFlowNodes((current) =>
        fitKnowledgeFolders(applyNodeChanges(changes, current)),
      );
      const state = useKnowledgeStore.getState();
      for (const change of changes) {
        if (
          change.type === "position" &&
          change.dragging === false &&
          change.position
        ) {
          void state.saveNode(change.id, {
            x: change.position.x,
            y: change.position.y,
          });
        }
        if (
          change.type === "dimensions" &&
          change.resizing === false &&
          change.dimensions &&
          state.graphExpanded.has(change.id)
        ) {
          void state.saveNode(change.id, {
            w: change.dimensions.width,
            h: change.dimensions.height,
          });
        }
      }
    },
    [setFlowNodes],
  );

  const seenStructure = useRef(structureNonce);
  useEffect(() => {
    const fresh = structureNonce !== seenStructure.current;
    seenStructure.current = structureNonce;
    setFlowNodes((current) => {
      const prevById = fresh
        ? new Map<string, KnowledgeFlowNodeType>()
        : new Map(current.map((node) => [node.id, node]));
      const folderByFlowId = new Map(
        laid.folders.map((folder) => [folder.id, folder]),
      );
      const folderNodes = [...laid.folders]
        .sort(
          (a, b) =>
            folderDepth(a.parentId, folderByFlowId) -
            folderDepth(b.parentId, folderByFlowId),
        )
        .map((folder) => {
          const prev = prevById.get(folder.id);
          if (prev?.type === "knowledgeFolder") return prev;
          return buildFolderNode(folder, prev);
        });
      const cardNodes = laid.nodes.map((node) => {
        const prev = prevById.get(node.id);
        const source = byId.get(node.nodeId);
        return buildFlowNode(
          node,
          focusedId,
          prev,
          graphExpanded.has(node.nodeId),
          source,
        );
      });
      return fitKnowledgeFolders([...folderNodes, ...cardNodes]);
    });
  }, [
    laid.folders,
    laid.nodes,
    focusedId,
    graphExpanded,
    byId,
    setFlowNodes,
    structureNonce,
  ]);

  const flowEdges = useMemo(
    () => toFlowEdges(laid.edges, colors, focusedId),
    [laid.edges, colors, focusedId],
  );

  return (
    <div className="relative h-full w-full">
      <ReactFlow
        nodes={flowNodes}
        edges={flowEdges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodeClick={(_, node) => {
          if (node.type !== "knowledge") return;
          selectFromGraph(node.id);
        }}
        onPaneClick={() => clearCanvasFocus()}
        onNodesChange={onNodesChange}
        nodesDraggable
        nodeDragThreshold={4}
        nodesConnectable={false}
        edgesReconnectable={false}
        elementsSelectable
        deleteKeyCode={null}
        minZoom={0.2}
        maxZoom={1.5}
        proOptions={{ hideAttribution: false }}
      >
        <Background color={colors.line} gap={18} size={1} />
        <Controls showInteractive={false} />
        <MiniMap
          pannable
          zoomable
          style={{ width: 140, height: 88 }}
          bgColor={colors.panel}
          maskColor={colors.mask}
          nodeColor={(node) => {
            const knowledge = byId.get(node.id);
            if (!knowledge) return colors.muted;
            if (nodeHasError(issues, knowledge.id)) return colors.red;
            if (knowledge.status === "pending") return colors.amber;
            if (knowledge.status === "disabled") return colors.muted;
            return colors.accent;
          }}
        />
        <FocusViewport />
      </ReactFlow>
    </div>
  );
}
