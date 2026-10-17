import { memo } from "react";

import { splitMarkdownBlocks } from "../lib/markdownBlocks";
import { useStreamingBuffer } from "../lib/streamingBuffer";
import { MarkdownBody, type MarkdownBodyProps } from "./MarkdownBody";

export const AgentMarkdown = memo(function AgentMarkdown({
  text,
  streaming = false,
  citations = false,
  inline = false,
  sourceId,
  preview,
}: MarkdownBodyProps) {
  const displayText = useStreamingBuffer(text, streaming);
  if (inline) {
    return (
      <MarkdownBody
        text={displayText}
        streaming={streaming}
        citations={citations}
        inline
        sourceId={sourceId}
        preview={preview}
      />
    );
  }

  const blocks = splitMarkdownBlocks(displayText);
  if (blocks.length <= 1) {
    return (
      <MarkdownBody
        text={displayText}
        streaming={streaming}
        citations={citations}
        sourceId={sourceId}
        preview={preview}
      />
    );
  }

  const last = blocks.length - 1;
  return blocks.map((block, index) => (
    <MarkdownBlock
      key={index}
      text={block}
      streaming={streaming && index === last}
      citations={citations}
      sourceId={sourceId}
      preview={preview}
      gap={index !== last}
    />
  ));
});

/**
 * One finished markdown block. The text of a closed block does not change
 * while the tail streams, so this stays mounted and skips re-parsing.
 */
const MarkdownBlock = memo(function MarkdownBlock({
  text,
  streaming,
  citations,
  sourceId,
  preview,
  gap,
}: MarkdownBodyProps & { gap: boolean }) {
  return (
    <div className={gap ? "mb-2.5" : undefined}>
      <MarkdownBody
        text={text}
        streaming={streaming}
        citations={citations}
        sourceId={sourceId}
        preview={preview}
      />
    </div>
  );
});
