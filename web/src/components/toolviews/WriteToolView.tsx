import { FileTextIcon } from "@phosphor-icons/react";

import { FilePathHeader } from "./FilePathHeader";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import { DiffView } from "./DiffView";
import { ToolResultBlock } from "./LspNote";
import type { ToolViewProps } from "./registry";

/**
 * Write-tool body: the written `content` rendered as a git-style diff (treated
 * as all additions, since the frontend has no prior file content) — visually
 * consistent with `EditToolView`. The tool result message and a collapsed
 * LSP-note tail (when the backend attached diagnostics) follow. Non-primary
 * fields (create_only) land in the infoicon.
 */
export function WriteToolView({ name, input, status, output }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const filePath =
    typeof obj.file_path === "string" ? obj.file_path : undefined;
  const content = typeof obj.content === "string" ? obj.content : "";

  const primary = TOOL_PARAM_META[name]?.primary ?? ["file_path", "content"];
  const meta = collectMetaFields(input, primary);

  const headerColor =
    status === "failed"
      ? "text-(--_dk-red-500)"
      : "text-(--_dk-text-secondary)";

  return (
    <div className="flex flex-col gap-1">
      <FilePathHeader
        icon={<FileTextIcon size={12} aria-hidden />}
        path={filePath}
        fallback="(no file_path)"
        colorClass={headerColor}
        meta={meta}
      />
      <DiffView oldText="" newText={content} />
      <ToolResultBlock output={output} />
    </div>
  );
}
