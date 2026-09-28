import {
  BaseEdge,
  EdgeLabelRenderer,
  getSmoothStepPath,
  type EdgeProps,
} from "@xyflow/react";

import type { KnowledgeEdgeVariant } from "../../lib/knowledge/layoutGraph";

export type KnowledgeRelationEdgeData = {
  variant: KnowledgeEdgeVariant;
  stroke: string;
  opacity: number;
};

export function KnowledgeRelationEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  markerEnd,
  data,
}: EdgeProps) {
  const edgeData = data as KnowledgeRelationEdgeData | undefined;
  const variant = edgeData?.variant ?? "solid";
  const stroke = edgeData?.stroke ?? "#777";
  const opacity = edgeData?.opacity ?? 1;
  const [edgePath, labelX, labelY] = getSmoothStepPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  });

  const dashed = variant !== "solid";

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        markerEnd={markerEnd as string | undefined}
        style={{
          stroke,
          strokeWidth: 1.25,
          strokeDasharray: dashed ? "5 4" : undefined,
          opacity,
        }}
      />
      {variant === "inactive" ? (
        <EdgeLabelRenderer>
          <div
            className="knowledge-edge-inactive-break nodrag nopan"
            style={{
              position: "absolute",
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
            }}
            aria-hidden
          >
            ×
          </div>
        </EdgeLabelRenderer>
      ) : null}
    </>
  );
}
