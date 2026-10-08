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
  const lines = String(node.attrs.lines ?? "").trim();
  const span = lines ? parseLineSpan(lines) : null;
  const citation: Citation =
    mode === "node"
      ? { kind: "node", key: id }
      : {
          kind: "file",
          path: id,
          symbol: symbol || null,
          line: span?.start ?? null,
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
