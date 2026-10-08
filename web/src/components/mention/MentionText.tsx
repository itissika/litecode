import {
  parseLineSpan,
  splitBodyRefs,
  type BodySegment,
} from "../../lib/knowledge/markers";
import type { Citation } from "../../lib/knowledge/refDisplay";
import { AgentMarkdown } from "../AgentMarkdown";
import { CitationChip } from "../CitationChip";

function citationFor(segment: Exclude<BodySegment, { type: "text" }>): Citation {
  if (segment.type === "ref") return { kind: "node", key: segment.id };
  const lines = segment.type === "symbol" ? segment.lines : null;
  const span = lines ? parseLineSpan(lines) : null;
  return {
    kind: "file",
    path: segment.path,
    symbol: segment.type === "symbol" ? segment.symbol : null,
    line: span?.start ?? null,
  };
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
  return <CitationChip citation={citationFor(segment)} />;
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
