import { memo } from "react";

import { splitMarkdownBlocks } from "../lib/markdownBlocks";
import { useStreamingBuffer } from "../lib/streamingBuffer";
import { MarkdownBody, type MarkdownBodyProps } from "./MarkdownBody";
import { WindowedMarkdown } from "./WindowedMarkdown";

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

  return (
    <WindowedMarkdown
      blocks={blocks}
      streaming={streaming}
      citations={citations}
      sourceId={sourceId}
      preview={preview}
    />
  );
});
