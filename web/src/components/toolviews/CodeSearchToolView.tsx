import { functionCallOutputText } from "../../api/adapter";
import { SyncPctHeader } from "./SyncPctHeader";
import type { ToolViewProps } from "./registry";
import { splitSyncPct } from "./syncPct";

const RAW_PRE =
  "whitespace-pre-wrap break-words font-mono text-dk-sm leading-relaxed";

/**
 * Expanded body of `code_search`. No dedicated hit grammar yet — Sync% head
 * (WP-057) then the remaining tool text. Missing sync_pct → omit the head.
 */
export function CodeSearchToolView({ status, output }: ToolViewProps) {
  const raw = output ? functionCallOutputText(output) : "";
  if (!raw) return null;
  const failed = status === "failed";
  const { sync, rest } = splitSyncPct(raw);
  const body = sync ? rest : raw;
  return (
    <div className="flex flex-col gap-1" data-testid="code-search-view">
      <SyncPctHeader sync={sync} />
      {body ? (
        <pre
          className={`${RAW_PRE} ${
            failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-secondary)"
          }`}
          data-testid="code-search-body"
        >
          {body}
        </pre>
      ) : null}
    </div>
  );
}
