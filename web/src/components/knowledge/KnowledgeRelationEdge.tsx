import { useEffect, useState } from "react";
import {
  EdgeLabelRenderer,
  getSmoothStepPath,
  type EdgeProps,
} from "@xyflow/react";

import type { KnowledgeEdgeVariant } from "../../lib/knowledge/layoutGraph";
import { prefersReducedMotion } from "../../lib/knowledge/flowProjection";

export type KnowledgeRelationEdgeData = {
  variant: KnowledgeEdgeVariant;
  stroke: string;
  opacity: number;
  /** False on the first paint of the canvas; later mounts draw from source to target. */
  draw?: boolean;
  leaving?: boolean;
};

const DRAW_MS = 320;

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
  const dashed = variant !== "solid";
  const animateIn = edgeData?.draw === true && edgeData.leaving !== true;
  const [phase, setPhase] = useState<"hold" | "draw" | "done">(() =>
    animateIn && !prefersReducedMotion() ? "hold" : "done",
  );
  const [edgePath, labelX, labelY] = getSmoothStepPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  });

  useEffect(() => {
    if (phase !== "hold") return;
    const frame = requestAnimationFrame(() => setPhase("draw"));
    return () => cancelAnimationFrame(frame);
  }, [phase]);

  const drawing = phase !== "done";
  const dashOffsetTransition =
    phase === "draw" ? `stroke-dashoffset ${DRAW_MS}ms cubic-bezier(0.45, 0, 0.2, 1), ` : "";

  return (
    <>
      <path
        id={id}
        d={edgePath}
        className="react-flow__edge-path"
        markerEnd={markerEnd as string | undefined}
        pathLength={drawing ? 1 : undefined}
        style={{
          stroke,
          strokeWidth: 0.75,
          opacity,
          fill: "none",
          strokeDasharray: drawing ? 1 : dashed ? "4 3" : undefined,
          strokeDashoffset: drawing ? (phase === "hold" ? 1 : 0) : undefined,
          transition: `${dashOffsetTransition}opacity 200ms ease`,
        }}
        onTransitionEnd={(event) => {
          if (event.propertyName !== "stroke-dashoffset") return;
          setPhase("done");
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
