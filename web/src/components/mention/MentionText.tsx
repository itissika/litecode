import type { MouseEvent } from "react";

import {
  normalizeKey,
  parseLineSpan,
  splitBodyRefs,
  type BodySegment,
} from "../../lib/knowledge/markers";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { AgentMarkdown } from "../AgentMarkdown";
import { WorkspaceCitationChip } from "../CitationChip";

function stopBubble(event: MouseEvent) {
  event.stopPropagation();
}

function ReadChip({
  className,
  label,
  title,
  onOpen,
}: {
  className: string;
  label: string;
  title?: string;
  onOpen?: () => void;
}) {
  return (
    <span className={className} title={title}>
      {onOpen ? (
        <button
          type="button"
          className="knowledge-token-label"
          aria-label={label}
          onMouseDown={stopBubble}
          onClick={(event) => {
            stopBubble(event);
            onOpen();
          }}
        >
          {label}
        </button>
      ) : (
        <span className="knowledge-token-label">{label}</span>
      )}
    </span>
  );
}

function NodeChip({ id, label }: { id: string; label: string }) {
  const key = normalizeKey(id);
  const target = useKnowledgeStore((state) => (key ? state.byKey.get(key) : undefined));
  const focusCanvas = useKnowledgeStore((state) => state.focusCanvas);
  return (
    <ReadChip
      className={target ? "knowledge-token" : "knowledge-token is-invalid"}
      label={label}
      onOpen={
        target
          ? () => {
              focusCanvas(target.id);
            }
          : undefined
      }
    />
  );
}

function FileChip({
  path,
  lines,
  symbol,
}: {
  path: string;
  label: string;
  lines: string | null;
  symbol: string | null;
}) {
  const span = lines ? parseLineSpan(lines) : null;
  return (
    <WorkspaceCitationChip path={path} symbol={symbol} line={span?.start ?? null} />
  );
}

const BLOCK_LINE = /^(?:#{1,6}\s|[-*+]\s|\d+\.\s|>\s?|```|~~~)/;

/** Text before the first blank line or block stays on the capsule's line. */
function partitionProse(value: string): { inline: string; block: string } {
  const lines = value.split("\n");
  let splitAt = lines.length;
  let pendingBlank = -1;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? "";
    if (line.trim() === "") {
      if (pendingBlank < 0) pendingBlank = i;
      continue;
    }
    if (BLOCK_LINE.test(line) || pendingBlank >= 0) {
      splitAt = pendingBlank >= 0 ? pendingBlank : i;
      break;
    }
  }
  if (splitAt >= lines.length) return { inline: value, block: "" };
  return {
    inline: lines.slice(0, splitAt).join("\n"),
    block: lines.slice(splitAt).join("\n"),
  };
}

function SegmentView({ segment }: { segment: BodySegment }) {
  if (segment.type === "text") {
    if (!segment.value) return null;
    const { inline, block } = partitionProse(segment.value);
    return (
      <>
        {inline ? <AgentMarkdown text={inline} inline streaming={false} /> : null}
        {block.trim() ? <AgentMarkdown text={block} streaming={false} /> : null}
      </>
    );
  }
  if (segment.type === "ref") return <NodeChip id={segment.id} label={segment.label} />;
  return (
    <FileChip
      path={segment.path}
      label={segment.label}
      lines={segment.type === "symbol" ? segment.lines : null}
      symbol={segment.type === "symbol" ? segment.symbol : null}
    />
  );
}

/** User prose with mention shortcodes drawn as read-only capsules. */
export function MentionText({ text }: { text: string }) {
  const segments = splitBodyRefs(text);
  if (!segments.some((segment) => segment.type !== "text")) {
    return <AgentMarkdown text={text} streaming={false} />;
  }
  return (
    <div className="mention-text">
      {segments.map((segment, index) => (
        <SegmentView key={index} segment={segment} />
      ))}
    </div>
  );
}
