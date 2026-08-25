import { GitDiffIcon } from "@phosphor-icons/react";

import { ToolInfoIcon } from "./InfoIcon";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import { DiffView } from "./DiffView";
import { ToolResultBlock } from "./LspNote";
import type { ToolViewProps } from "./registry";

export interface EditBlockPreview {
  oldString: string;
  newString: string;
  replaceAll: boolean;
}

/**
 * New calls use `edits[]`. Historical messages still have top-level
 * `old_string` / `new_string`. Request diffs are previews, not apply status.
 */
export function collectEditBlocks(input: unknown): EditBlockPreview[] {
  if (!input || typeof input !== "object" || Array.isArray(input)) return [];
  const obj = input as Record<string, unknown>;
  if (Array.isArray(obj.edits)) {
    return obj.edits.flatMap((item) => {
      if (!item || typeof item !== "object" || Array.isArray(item)) return [];
      const rec = item as Record<string, unknown>;
      return [
        {
          oldString: typeof rec.old_string === "string" ? rec.old_string : "",
          newString: typeof rec.new_string === "string" ? rec.new_string : "",
          replaceAll: rec.replace_all === true,
        },
      ];
    });
  }
  if (typeof obj.old_string === "string" || typeof obj.new_string === "string") {
    return [
      {
        oldString: typeof obj.old_string === "string" ? obj.old_string : "",
        newString: typeof obj.new_string === "string" ? obj.new_string : "",
        replaceAll: obj.replace_all === true,
      },
    ];
  }
  return [];
}

/**
 * Edit-tool body: a git-like unified diff of each requested change (computed
 * from the tool input — no backend data required), plus the tool result and a
 * collapsed LSP/warning tail. Failed calls tint the header red via `status`;
 * partial success is a warning, not a failure.
 */
export function EditToolView({ name, input, status, output }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const filePath = typeof obj.file_path === "string" ? obj.file_path : undefined;
  const blocks = collectEditBlocks(input);
  const primary = Array.isArray(obj.edits)
    ? ["file_path", "edits"]
    : (TOOL_PARAM_META[name]?.primary ?? ["file_path", "old_string", "new_string"]);
  const meta = collectMetaFields(input, primary);

  const headerColor =
    status === "failed"
      ? "text-(--_dk-red-500)"
      : status === "warning"
        ? "text-(--_dk-amber-500)"
        : "text-(--_dk-text-secondary)";

  const countLabel =
    blocks.length > 1 ? `${blocks.length} edits` : blocks.length === 1 ? "1 edit" : "request preview";

  return (
    <div className="flex flex-col gap-1">
      <div
        className={`flex items-center gap-1.5 text-dk-xs font-medium ${headerColor}`}
      >
        <GitDiffIcon size={12} aria-hidden />
        <span className="min-w-0 flex-1 truncate font-mono">
          {filePath ?? "(unknown file)"}
        </span>
        <span className="shrink-0 text-(--_dk-text-muted)">{countLabel}</span>
        <ToolInfoIcon fields={meta} />
      </div>
      {blocks.length === 0 ? (
        <div className="text-dk-xs text-(--_dk-text-muted)">No edit preview</div>
      ) : (
        blocks.map((block, index) => (
          <div key={index} className="flex flex-col gap-1">
            {blocks.length > 1 && (
              <div className="text-dk-xs text-(--_dk-text-muted)">
                edit {index + 1}
                {block.replaceAll ? " · replace_all" : ""}
                <span className="ml-1 opacity-70">request preview</span>
              </div>
            )}
            {blocks.length === 1 && block.replaceAll && (
              <div className="text-dk-xs text-(--_dk-text-muted)">replace_all · request preview</div>
            )}
            <DiffView oldText={block.oldString} newText={block.newString} />
          </div>
        ))
      )}
      <ToolResultBlock output={output} />
    </div>
  );
}
