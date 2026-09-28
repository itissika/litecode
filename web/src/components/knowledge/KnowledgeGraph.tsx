import { ArrowsOutSimple, Folder, WarningCircle } from "@phosphor-icons/react";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  Background,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  NodeResizeControl,
  applyNodeChanges,
  useReactFlow,
  useNodesState,
  Handle,
  Position,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
} from "@xyflow/react";

import { FoldCard } from "../FoldCard";
import {
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_MAX_HEIGHT,
  KNOWLEDGE_NODE_MAX_WIDTH,
  KNOWLEDGE_NODE_MIN_HEIGHT,
  KNOWLEDGE_NODE_MIN_WIDTH,
  KNOWLEDGE_NODE_WIDTH,
  layoutKnowledgeGraph,
  type LaidOutEdge,
} from "../../lib/knowledge/layoutGraph";
import { normalizeKey } from "../../lib/knowledge/markers";
import { fitKnowledgeFolders } from "../../lib/knowledge/fitFolders";
import { knowledgeListAlert, nodeHasError } from "../../lib/knowledge/validate";
import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import {
  KnowledgeInlineBody,
  KnowledgeMarkdown,
} from "./KnowledgeMarkdown";
import { KnowledgeRelationEdge } from "./KnowledgeRelationEdge";

const NO_ISSUES: KnowledgeIssue[] = [];

type KnowledgeNodeData = { nodeId: number };
type KnowledgeFolderData = { folderId: number };
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

function defaultNodeStyle(): CSSProperties {
  return {
    width: KNOWLEDGE_NODE_WIDTH,
    height: KNOWLEDGE_NODE_HEIGHT,
  };
}

function nodeStyleFromPrev(prev: KnowledgeFlowNodeType | undefined): CSSProperties {
  if (!prev?.style) return defaultNodeStyle();
  return {
    width: cssDimension(prev.style.width, KNOWLEDGE_NODE_WIDTH),
    height: cssDimension(prev.style.height, KNOWLEDGE_NODE_HEIGHT),
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

const KnowledgeFlowNode = memo(function KnowledgeFlowNode({
  data,
}: NodeProps<Node<KnowledgeNodeData, "knowledge">>) {
  const node = useKnowledgeStore((s) => s.byId.get(data.nodeId));
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const focusNode = useKnowledgeStore((s) =>
    focusedId == null ? undefined : s.byId.get(focusedId),
  );
  const graphOpen = useKnowledgeStore((s) => s.graphExpanded.has(data.nodeId));
  const toggleGraph = useKnowledgeStore((s) => s.toggleGraph);
  const issues = useKnowledgeStore(
    (s) => s.issuesByNode.get(data.nodeId) ?? NO_ISSUES,
  );

  if (!node) return null;

  const listAlert = knowledgeListAlert(issues, node.status);
  const disabled = node.status === "disabled";

  const focused = focusedId === node.id;
  let dimmed = false;
  if (focusedId != null && !focused && focusNode) {
    const linked =
      focusNode.relations.includes(node.id) ||
      node.relations.includes(focusedId);
    dimmed = !linked;
  }

  const key = normalizeKey(node.key);

  return (
    <>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <div
        className={[
          "knowledge-flow-card",
          graphOpen ? "is-expanded" : "",
          focused ? "is-focused" : "",
          dimmed ? "is-dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {node.status === "disabled" && !focused ? (
          <div className="knowledge-flow-disabled-veil" aria-hidden />
        ) : null}
        <FoldCard
          className="knowledge-flow-foldcard"
          showArrow={false}
          summaryMode
          instantBody
          edgeBlur={false}
          open={graphOpen}
          onToggle={(next) => {
            if (next === graphOpen) return;
            toggleGraph(node.id);
          }}
          label={
            <span className="flex min-w-0 items-center gap-1">
              {listAlert ? (
                <WarningCircle
                  size={12}
                  weight="fill"
                  className={
                    listAlert === "red"
                      ? "knowledge-list-alert is-red"
                      : "knowledge-list-alert is-amber"
                  }
                  aria-label={
                    listAlert === "red"
                      ? "Reference problem"
                      : "Pending review"
                  }
                />
              ) : null}
              <span
                className={[
                  "knowledge-card-title knowledge-node-title truncate font-mono",
                  disabled ? "is-disabled" : "text-(--_dk-text-primary)",
                ]
                  .filter(Boolean)
                  .join(" ")}
              >
                {key}
              </span>
            </span>
          }
          headerAriaLabel={key}
          summary={
            <KnowledgeInlineBody sourceId={node.id} text={node.value} />
          }
          frameColor="transparent"
          contentClassName="knowledge-markdown"
        >
          <KnowledgeMarkdown sourceId={node.id} text={node.value} />
        </FoldCard>
        <NodeResizeControl
          position="bottom-right"
          minWidth={KNOWLEDGE_NODE_MIN_WIDTH}
          minHeight={KNOWLEDGE_NODE_MIN_HEIGHT}
          maxWidth={KNOWLEDGE_NODE_MAX_WIDTH}
          maxHeight={KNOWLEDGE_NODE_MAX_HEIGHT}
          className="knowledge-flow-resize-handle"
        >
          <ArrowsOutSimple size={10} weight="bold" aria-hidden />
        </NodeResizeControl>
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </>
  );
});

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
  knowledge: KnowledgeFlowNode,
  knowledgeFolder: KnowledgeFolderNode,
};
const edgeTypes = { knowledgeRelation: KnowledgeRelationEdge };

function buildFlowNode(
  node: ReturnType<typeof layoutKnowledgeGraph>["nodes"][number],
  focusedId: number | null,
  prev: KnowledgeFlowNodeType | undefined,
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
    style: nodeStyleFromPrev(prev),
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
  focusedId: number | null,
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
    },
    [setFlowNodes],
  );

  useEffect(() => {
    setFlowNodes((current) => {
      const prevById = new Map(current.map((node) => [node.id, node]));
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
        const zIndex = node.nodeId === focusedId ? 10 : 1;
        if (prev?.type === "knowledge" && prev.zIndex === zIndex) return prev;
        return buildFlowNode(node, focusedId, prev);
      });
      return fitKnowledgeFolders([...folderNodes, ...cardNodes]);
    });
  }, [laid.folders, laid.nodes, focusedId, setFlowNodes]);

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
          const id = Number(node.id);
          if (Number.isInteger(id)) selectFromGraph(id);
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
            const knowledge = byId.get(Number(node.id));
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
