import {
  BrainIcon,
  PencilIcon,
  TerminalIcon,
  WrenchIcon,
} from "@phosphor-icons/react";

import { functionCallOutputText } from "../../api/adapter";
import { isBackgroundBashResult } from "../../lib/bashLive";
import { foldId, type RenderNode } from "../../lib/transcriptProjection";
import { isInlineCall, processToolBucket } from "../../lib/toolCategory";
import { useEditorStore } from "../../stores/editorStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useTurnStore } from "../../stores/turnStore";
import { AgentMarkdown } from "../AgentMarkdown";
import { ImageThumb } from "../ImageThumb";
import { CategoryCount } from "../CategoryCount";
import { FoldCard } from "../FoldCard";
import { InlineToolRow } from "../InlineToolRow";
import { ToolCallCard } from "../ToolCallCard";
import { TranscriptMark } from "../transcriptMarks";

export function NodeView({
  node,
  streaming = false,
  projectRoot,
  onOpenFile,
  sessionId,
  bubbleKey,
  citations = false,
}: {
  node: RenderNode;
  streaming?: boolean;
  projectRoot?: string | null;
  onOpenFile?: (path: string) => void;
  sessionId?: string;
  /** Stable bubble identity (projection key of the bubble's first row), used to
   *  namespace this node's FoldCard state across virtual-list remounts. */
  bubbleKey?: string;
  /** Turn file and web links in assistant prose into citation chips. */
  citations?: boolean;
}) {
  // The plan-execution mark names the plan the button launched; the row itself
  // only carries the prompt text, so the path comes from the session pointer.
  const activePlanPath = useTurnStore((s) =>
    sessionId ? (s.byId.get(sessionId)?.activePlanPath ?? null) : null,
  );
  switch (node.kind) {
    case "reasoning":
      return (
        <FoldCard
          id={
            bubbleKey && sessionId
              ? foldId(sessionId, bubbleKey, `reasoning:${node.key}`)
              : undefined
          }
          className="text-sm"
          contentClassName="text-(--_dk-text-secondary)"
          icon={
            <BrainIcon
              size={13}
              aria-hidden
              className="shrink-0 text-(--_dk-text-muted)"
            />
          }
          label={node.incomplete ? "Reasoning (incomplete)" : "Reasoning"}
          autoOpen={node.live}
          streaming={streaming}
        >
          <AgentMarkdown
            text={node.text}
            streaming={streaming}
            citations={citations}
          />
        </FoldCard>
      );
    case "images":
      return (
        <div className="mb-1 flex flex-wrap gap-1.5 pl-(--_dk-indent-card-head)">
          {node.refs.map((ref, index) => (
            <ImageThumb key={`${ref}:${index}`} mediaRef={ref} />
          ))}
        </div>
      );
    case "text":
      return (
        <div className="text-dk-base text-(--_dk-text-primary) pl-(--_dk-indent-card-head)">
          <AgentMarkdown
            text={node.text}
            streaming={streaming}
            citations={citations}
          />
          {node.incomplete && !streaming ? (
            <div className="mt-1 text-dk-2xs italic text-(--_dk-text-disabled)">
              Output incomplete
            </div>
          ) : null}
        </div>
      );
    case "tool": {
      // Background bash is the one call that goes inline on its RESULT rather
      // than its name; foreground bash keeps the rich card.
      const backgroundBash =
        node.call.name === "bash" &&
        isBackgroundBashResult(
          node.output ? functionCallOutputText(node.output) : "",
        );
      if (isInlineCall(node.call.name, backgroundBash)) {
        return (
          <InlineToolRow
            call={node.call}
            output={node.output}
            streaming={node.live}
            sessionId={sessionId}
          />
        );
      }
      return (
        <ToolCallCard
          call={node.call}
          output={node.output}
          streaming={node.live}
          projectRoot={projectRoot ?? null}
          onOpenFile={(path) => onOpenFile?.(path)}
          sessionId={sessionId}
          foldCardId={
            bubbleKey && sessionId
              ? foldId(sessionId, bubbleKey, `tool:${node.call.call_id}`)
              : undefined
          }
        />
      );
    }
    case "compact_cut":
      return <TranscriptMark kind={node.kind} summary={node.summary} />;
    case "job_exit":
      return <TranscriptMark kind={node.kind} detail={node.detail} />;
    case "subagent_exit":
      return (
        <TranscriptMark
          kind={node.kind}
          detail={node.detail}
          childId={node.childId}
        />
      );
    case "plan":
      return <TranscriptMark kind={node.kind} />;
    case "plan_execute":
      return <TranscriptMark kind={node.kind} planPath={activePlanPath} />;
  }
}

export function ProcessGroup({
  nodes,
  streaming,
  autoOpen,
  sessionId,
  bubbleKey,
  groupIndex,
}: {
  nodes: RenderNode[];
  streaming: boolean;
  autoOpen: boolean;
  sessionId?: string;
  /** Stable bubble identity, used to namespace this group's FoldCard state. */
  bubbleKey?: string;
  /** Index of this process group within its bubble (for a unique FoldCard id). */
  groupIndex: number;
}) {
  const project = useSessionStore((s) => s.project);
  const openFile = useEditorStore((s) => s.openFile);

  const reasoningCount = nodes.filter((n) => n.kind === "reasoning").length;
  let bashCount = 0;
  let editCount = 0;
  let toolCount = 0;
  for (const node of nodes) {
    if (node.kind !== "tool") continue;
    const bucket = processToolBucket(node.call.name);
    if (bucket === "bash") bashCount += 1;
    else if (bucket === "edit") editCount += 1;
    else if (bucket === "tool") toolCount += 1;
  }

  const ariaParts: string[] = [];
  if (reasoningCount > 0) {
    ariaParts.push(`${reasoningCount} reasoning`);
  }
  if (bashCount > 0) {
    ariaParts.push(`${bashCount} bash`);
  }
  if (editCount > 0) {
    ariaParts.push(`${editCount} edit`);
  }
  if (toolCount > 0) {
    ariaParts.push(`${toolCount} tool${toolCount !== 1 ? "s" : ""}`);
  }
  const headerAriaLabel = ariaParts.join(", ") || "Process";

  return (
    <FoldCard
      id={
        bubbleKey && sessionId
          ? foldId(sessionId, bubbleKey, `process:${groupIndex}`)
          : undefined
      }
      icon={null}
      headerClassName="text-dk-sm text-(--_dk-text-secondary)"
      label={
        <span className="flex min-w-0 flex-1 items-center gap-2.5">
          <CategoryCount
            icon={
              <BrainIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-text-secondary)"
              />
            }
            count={reasoningCount}
            noun="reasoning"
          />
          <CategoryCount
            icon={
              <TerminalIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-text-secondary)"
              />
            }
            count={bashCount}
            noun="bash"
          />
          <CategoryCount
            icon={
              <PencilIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-text-secondary)"
              />
            }
            count={editCount}
            noun="edit"
          />
          <CategoryCount
            icon={
              <WrenchIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-amber-500)"
              />
            }
            count={toolCount}
            noun="tool"
          />
        </span>
      }
      headerAriaLabel={headerAriaLabel}
      autoOpen={autoOpen}
      streaming={streaming}
    >
      <div className="space-y-1">
        {nodes.map((node) => (
          <NodeView
            key={node.key}
            node={node}
            streaming={node.streaming}
            projectRoot={project}
            onOpenFile={(path) => void openFile(path)}
            sessionId={sessionId}
            bubbleKey={bubbleKey}
            citations
          />
        ))}
      </div>
    </FoldCard>
  );
}
