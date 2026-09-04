import { FileTextIcon } from "@phosphor-icons/react";

import { ToolInfoIcon } from "./InfoIcon";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import type { ToolViewProps } from "./registry";

/**
 * Input view for `file_path`-style tools (`read` / `write`). The `file_path` is
 * shown prominently; `write`'s `content` is rendered as a code block payload.
 * All other fields (read: start_line/end_line/token_budget, write: create_only) are
 * tucked into the infoicon. Output is not specialized — it flows through the
 * default renderer in `ToolContentView`.
 */
export function FileParamView({ name, status, input }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const filePath =
    typeof obj.file_path === "string" ? obj.file_path : undefined;
  const content = typeof obj.content === "string" ? obj.content : undefined;

  const primary = TOOL_PARAM_META[name]?.primary ?? ["file_path"];
  const meta = collectMetaFields(input, primary);

  const headerColor =
    status === "failed"
      ? "text-(--_dk-red-500)"
      : "text-(--_dk-text-secondary)";

  return (
    <div className="flex flex-col gap-1">
      <div className={`flex items-center gap-1.5 text-dk-xs font-medium ${headerColor}`}>
        <FileTextIcon size={12} aria-hidden />
        <span className="min-w-0 flex-1 truncate font-mono">
          {filePath ?? "(no file_path)"}
        </span>
        <ToolInfoIcon fields={meta} />
      </div>
      {content !== undefined && (
        <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md border border-(--_dk-line-visible) bg-(--_dk-surface-raised) px-2 py-1.5 font-mono text-dk-sm leading-relaxed text-(--_dk-text-secondary)">
          {content}
        </pre>
      )}
    </div>
  );
}
