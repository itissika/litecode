import { FileTextIcon } from "@phosphor-icons/react";

import { FilePathHeader } from "./FilePathHeader";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import type { ToolViewProps } from "./registry";

/**
 * Input view for `file_path`-style tools (`read` / `write`). The `file_path` is
 * shown prominently; `write`'s `content` is rendered as a code block payload.
 * All other fields (read: start_line/end_line, write: create_only) are
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
      <FilePathHeader
        icon={<FileTextIcon size={12} aria-hidden />}
        path={filePath}
        fallback="(no file_path)"
        colorClass={headerColor}
        meta={meta}
      />
      {content !== undefined && (
        <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md border border-(--_dk-line-visible) bg-(--_dk-surface-raised) px-2 py-1.5 font-mono text-dk-sm leading-relaxed text-(--_dk-text-secondary)">
          {content}
        </pre>
      )}
    </div>
  );
}
