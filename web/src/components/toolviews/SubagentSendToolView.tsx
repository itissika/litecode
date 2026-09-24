import { functionCallOutputText } from "../../api/adapter";
import {
  inputString,
  sendStatusWord,
  truncateLine,
} from "../../lib/subagentUi";
import { useSessionStore } from "../../stores/sessionStore";
import type { ToolViewProps } from "./registry";
import { SubagentInlineLine } from "./SubagentInlineLine";

/**
 * Single-line `subagent_send`: child agent + truncated message + live status.
 * The tool's `format_started` body stays in the log for the agent, not the row.
 */
export function SubagentSendToolView({ input, output, status }: ToolViewProps) {
  const childId = inputString(input, "id");
  const message = inputString(input, "message");
  const child = useSessionStore((s) =>
    childId ? s.sessions.find((session) => session.id === childId) : undefined,
  );
  const label = child?.agent_id || (childId ? childId.slice(0, 8) : "subagent");
  const raw = output ? functionCallOutputText(output) : undefined;
  const statusText = sendStatusWord(child, status, raw);
  return (
    <SubagentInlineLine
      label={label}
      secondary={message ? truncateLine(message) : undefined}
      statusText={statusText}
      failed={status === "failed" || statusText === "error"}
      childId={childId}
      testId="subagent-send-line"
    />
  );
}
