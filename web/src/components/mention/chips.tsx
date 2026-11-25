import { NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";

import { parseLineSpan } from "../../lib/knowledge/markers";
import type { Citation } from "../../lib/knowledge/refDisplay";
import { CitationChip } from "../CitationChip";

/** Single TipTap mention node view. Knowledge nodes, files, symbols, and ranges share one capsule. */
export function MentionChipView({
  node,
  deleteNode,
  sourceIdRef,
}: ReactNodeViewProps & {
  sourceIdRef: { current: string | undefined };
}) {
  const path = String(node.attrs.id ?? "");
  const label = node.attrs.label == null ? undefined : String(node.attrs.label);
  const symbol = String(node.attrs.symbol ?? "").trim();
  const lines = node.attrs.lines == null ? null : String(node.attrs.lines);
  const badLines = node.attrs.badLines == null ? null : String(node.attrs.badLines);
  const span = !symbol && lines ? parseLineSpan(lines.trim()) : null;
  const citation: Citation = {
    path,
    symbol: symbol || null,
    line: span?.start ?? null,
    lines: symbol ? null : lines,
    badLines: symbol || lines ? null : badLines,
    label: label ?? null,
  };

  return (
    <NodeViewWrapper as="span">
      <CitationChip
        citation={citation}
        sourceId={sourceIdRef.current}
        label={label}
        onRemove={deleteNode}
        nodrag
      />
    </NodeViewWrapper>
  );
}
