import { functionCallOutputText } from "../../api/adapter";
import { waitSettledLine, waitTargetCount } from "../../lib/subagentUi";
import { WaveText } from "../WaveText";
import type { ToolViewProps } from "./registry";

const waitLineClass = "font-mono text-dk-sm";

/**
 * Single-line `subagent_wait`: ids/count while pending, settled N once the
 * Session barrier returns. Full reports stay in the log for the agent.
 */
export function SubagentWaitToolView({
  input,
  output,
  status,
}: ToolViewProps) {
  if (status === "failed") {
    return (
      <div className={`${waitLineClass} text-(--_dk-red-500)`} data-testid="subagent-wait-line">
        wait failed
      </div>
    );
  }

  if (output) {
    return (
      <div className={`${waitLineClass} text-(--_dk-text-muted)`} data-testid="subagent-wait-line">
        {waitSettledLine(functionCallOutputText(output))}
      </div>
    );
  }

  const target = waitTargetCount(input);
  return (
    <div className={waitLineClass} data-testid="subagent-wait-pending">
      <WaveText text={target ? `waiting ${target}…` : "waiting…"} />
    </div>
  );
}
