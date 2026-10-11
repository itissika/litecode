import { NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";

import { parseLineSpan } from "../../lib/knowledge/markers";
import type { Citation } from "../../lib/knowledge/refDisplay";
import { CitationChip } from "../CitationChip";

/** TipTap mention. Both node and file shortcodes render the same capsule. */
export function MentionChipView({
  node,
  deleteNode,
  sourceIdRef,
  mode,
}: ReactNodeViewProps & {
  sourceIdRef: { current: string | undefined };
  mode: "node" | "file";
}) {
  const id = String(node.attrs.id ?? "");
  const symbol = String(node.attrs.symbol ?? "").trim();
  const rawLines = node.attrs.lines == null ? null : String(node.attrs.lines);
  // A symbol citation never carries lines; only a line range opens at a line.
  const span = !symbol && rawLines ? parseLineSpan(rawLines.trim()) : null;
  const citation: Citation =
    mode === "node"
      ? { kind: "node", key: id }
      : {
          kind: "file",
          path: id,
          symbol: symbol || null,
          line: span?.start ?? null,
          lines: symbol ? null : rawLines,
        };

  return (
    <NodeViewWrapper as="span">
      <CitationChip
        citation={citation}
        sourceId={sourceIdRef.current}
        onRemove={deleteNode}
        nodrag
      />
    </NodeViewWrapper>
  );
}
