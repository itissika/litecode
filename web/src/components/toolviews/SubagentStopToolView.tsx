import { functionCallOutputText } from "../../api/adapter";
import { inputString, stopStatusWord } from "../../lib/subagentUi";
import { useSessionStore } from "../../stores/sessionStore";
import type { ToolViewProps } from "./registry";
import { SubagentInlineLine } from "./SubagentInlineLine";

/**
 * Single-line `subagent_stop`: child agent + request vs already-ended vs idle.
 */
export function SubagentStopToolView({ input, output, status }: ToolViewProps) {
  const childId = inputString(input, "id");
  const child = useSessionStore((s) =>
    childId ? s.sessions.find((session) => session.id === childId) : undefined,
  );
  const label = child?.agent_id || (childId ? childId.slice(0, 8) : "subagent");
  const raw = output ? functionCallOutputText(output) : undefined;
  return (
    <SubagentInlineLine
      label={label}
      statusText={stopStatusWord(raw, status)}
      failed={status === "failed"}
      childId={childId}
      testId="subagent-stop-line"
    />
  );
}
