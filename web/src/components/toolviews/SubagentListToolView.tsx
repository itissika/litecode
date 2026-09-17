import { functionCallOutputText } from "../../api/adapter";
import { listStatusWord } from "../../lib/subagentUi";
import type { ToolViewProps } from "./registry";

/** Single-line `subagent_list`: session count. The roster is the detailed view. */
export function SubagentListToolView({ output, status }: ToolViewProps) {
  const raw = output ? functionCallOutputText(output) : undefined;
  const failed = status === "failed";
  return (
    <div
      className={`font-mono text-dk-sm ${
        failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-muted)"
      }`}
      data-testid="subagent-list-line"
    >
      {listStatusWord(raw, status)}
    </div>
  );
}
