import type { MouseEvent } from "react";

import { humanFileLabel, humanSymbolLabel } from "../lib/knowledge/markers";
import { useEditorStore } from "../stores/editorStore";

function stopChipEvent(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
}

/** Read-only file or symbol capsule. Same shape as a mention the user typed. */
export function WorkspaceCitationChip({
  path,
  symbol = null,
  line = null,
}: {
  path: string;
  symbol?: string | null;
  line?: number | null;
}) {
  const chain = symbol?.trim() ?? "";
  const text = chain ? humanSymbolLabel(path, chain) : humanFileLabel(path);
  const className = chain ? "knowledge-token is-file is-symbol" : "knowledge-token is-file";
  const open = () => {
    const editor = useEditorStore.getState();
    if (line != null) void editor.openFileAt(path, line);
    else void editor.openFile(path);
  };
  return (
    <span className={className} title={path}>
      <button
        type="button"
        className="knowledge-token-label"
        aria-label={text}
        onMouseDown={stopChipEvent}
        onClick={(event) => {
          stopChipEvent(event);
          open();
        }}
      >
        {text}
      </button>
    </span>
  );
}
