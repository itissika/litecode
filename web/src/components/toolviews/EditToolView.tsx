import { GitDiffIcon } from "@phosphor-icons/react";

import { ToolInfoIcon } from "./InfoIcon";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import { DiffView } from "./DiffView";
import { ToolResultBlock } from "./LspNote";
import type { ToolViewProps } from "./registry";

/**
 * Edit-tool body: a git-like unified diff of the change (computed from the
 * tool's own old_string/new_string input — no backend data required), plus the
 * tool result message and a collapsed LSP-note tail when the backend attached
 * diagnostics. Failed calls tint the header red via `status`. Non-primary fields
 * (replace_all / mode) are tucked into the infoicon.
 */
export function EditToolView({ name, input, status, output }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const filePath = typeof obj.file_path === "string" ? obj.file_path : undefined;
  const oldString = typeof obj.old_string === "string" ? obj.old_string : "";
  const newString = typeof obj.new_string === "string" ? obj.new_string : "";

  const primary = TOOL_PARAM_META[name]?.primary ?? [
    "file_path",
    "old_string",
    "new_string",
  ];
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
        <GitDiffIcon size={12} aria-hidden />
        <span className="min-w-0 flex-1 truncate font-mono">
          {filePath ?? "(unknown file)"}
        </span>
        <ToolInfoIcon fields={meta} />
      </div>
      <DiffView oldText={oldString} newText={newString} />
      <ToolResultBlock output={output} />
    </div>
  );
}
