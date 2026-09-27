import { useEffect, useMemo, useState } from "react";
import {
  Background,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  useReactFlow,
  Handle,
  Position,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";

import {
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_WIDTH,
  layoutKnowledgeGraph,
  type LaidOutEdge,
} from "../../lib/knowledge/layoutGraph";
import { knowledgePreview, normalizeKey } from "../../lib/knowledge/markers";
import { nodeHasError } from "../../lib/knowledge/validate";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeStatusBadge } from "./KnowledgeMarkdown";

type KnowledgeNodeData = { nodeId: number };
type KnowledgeFlowNodeType = Node<KnowledgeNodeData, "knowledge">;

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

function KnowledgeFlowNode({ data }: NodeProps<KnowledgeFlowNodeType>) {
  const node = useKnowledgeStore((s) => s.byId.get(data.nodeId));
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const nodes = useKnowledgeStore((s) => s.nodes);
  const hasError = useKnowledgeStore((s) => nodeHasError(s.issues, data.nodeId));
  if (!node) return null;

  const focused = focusedId === node.id;
  let dimmed = false;
  if (focusedId != null && !focused) {
    const current = nodes.find((item) => item.id === focusedId);
    const linked =
      current != null &&
      (current.relations.includes(node.id) ||
        node.relations.includes(focusedId));
    dimmed = !linked;
  }

  return (
    <>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <div
        className={[
          "knowledge-flow-card",
          node.status === "pending" ? "is-pending" : "",
          node.status === "disabled" && !focused ? "is-disabled" : "",
          hasError ? "has-error" : "",
          focused ? "is-focused" : "",
          dimmed ? "is-dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ")}
      >
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="truncate font-mono text-(--_dk-text-primary)">
            {normalizeKey(node.key)}
          </span>
          <span className="shrink-0 text-(--_dk-text-muted)">#{node.id}</span>
          <KnowledgeStatusBadge status={node.status} />
        </div>
        <p className="knowledge-flow-preview">{knowledgePreview(node.value)}</p>
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </>
  );
}

const nodeTypes = { knowledge: KnowledgeFlowNode };

function toFlowNodes(
  laidOut: ReturnType<typeof layoutKnowledgeGraph>["nodes"],
  focusedId: number | null,
): KnowledgeFlowNodeType[] {
  return laidOut.map((node) => ({
    id: node.id,
    type: "knowledge",
    position: { x: node.x, y: node.y },
    data: { nodeId: node.nodeId },
    draggable: false,
    connectable: false,
    zIndex: node.nodeId === focusedId ? 10 : 0,
    style: { width: KNOWLEDGE_NODE_WIDTH, height: KNOWLEDGE_NODE_HEIGHT },
  }));
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
    const stroke = edge.warning ? colors.amber : colors.muted;
    return {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      type: "smoothstep",
      style: {
        stroke,
        strokeWidth: 1.25,
        strokeDasharray: edge.warning ? "5 4" : undefined,
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

function FocusViewport({
  positions,
}: {
  positions: Map<number, { x: number; y: number }>;
}) {
  const { setCenter } = useReactFlow();
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const nonce = useKnowledgeStore((s) => s.focusNonce);
  useEffect(() => {
    const id = focusedId ?? positions.keys().next().value;
    if (id == null) return;
    const pos = positions.get(id);
    if (!pos) return;
    const timer = window.setTimeout(() => {
      void setCenter(
        pos.x + KNOWLEDGE_NODE_WIDTH / 2,
        pos.y + KNOWLEDGE_NODE_HEIGHT / 2,
        { zoom: 1, duration: nonce === 0 ? 0 : 280 },
      );
    }, 40);
    return () => window.clearTimeout(timer);
  }, [focusedId, nonce, positions, setCenter]);
  return null;
}

export function KnowledgeGraph() {
  const nodes = useKnowledgeStore((s) => s.nodes);
  const issues = useKnowledgeStore((s) => s.issues);
  const byId = useKnowledgeStore((s) => s.byId);
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const focus = useKnowledgeStore((s) => s.focus);
  const colors = useGraphColors();
  const laid = useMemo(
    () => layoutKnowledgeGraph(nodes, issues),
    [nodes, issues],
  );
  const positions = useMemo(() => {
    const map = new Map<number, { x: number; y: number }>();
    for (const node of laid.nodes) map.set(node.nodeId, { x: node.x, y: node.y });
    return map;
  }, [laid.nodes]);
  const flowNodes = useMemo(
    () => toFlowNodes(laid.nodes, focusedId),
    [laid.nodes, focusedId],
  );
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
        onNodeClick={(_, node) => {
          const id = Number(node.id);
          if (Number.isInteger(id)) focus(id);
        }}
        nodesDraggable={false}
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
        <FocusViewport positions={positions} />
      </ReactFlow>
      <div className="knowledge-legend" aria-hidden>
        <span>
          <i className="knowledge-legend-line" />
          引用
        </span>
        <span>
          <i className="knowledge-legend-line is-warning" />
          警告
        </span>
      </div>
    </div>
  );
}
