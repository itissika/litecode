import { FileTextIcon } from "@phosphor-icons/react";

import { ToolInfoIcon } from "./InfoIcon";
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
  const filePath = typeof obj.file_path === "string" ? obj.file_path : undefined;
  const content = typeof obj.content === "string" ? obj.content : "";

  const primary = TOOL_PARAM_META[name]?.primary ?? ["file_path", "content"];
  const meta = collectMetaFields(input, primary);

  const headerColor =
    status === "failed"
      ? "text-(--_dk-red-500)"
      : "text-(--_dk-text-secondary)";

  return (
    <div className="flex flex-col gap-1">
      <div
        className={`flex items-center gap-1.5 text-dk-xs font-medium ${headerColor}`}
      >
        <FileTextIcon size={12} aria-hidden />
        <span className="min-w-0 flex-1 truncate font-mono">
          {filePath ?? "(no file_path)"}
        </span>
        <ToolInfoIcon fields={meta} />
      </div>
      <DiffView oldText="" newText={content} />
      <ToolResultBlock output={output} />
    </div>
  );
}
