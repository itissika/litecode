import { childStatusWord, inputString } from "../../lib/subagentUi";
import { useMessageStore } from "../../stores/messageStore";
import { useSessionStore } from "../../stores/sessionStore";
import type { ToolViewProps } from "./registry";
import { SubagentInlineLine } from "./SubagentInlineLine";

/**
 * Single-line inline view for `subagent_launch`: agent + optional
 * responsibility + live child Session status.
 */
export function SubagentLaunchToolView({
  input,
  status,
  call_id,
  sessionId,
}: ToolViewProps) {
  const agent = inputString(input, "agent") ?? "subagent";
  const responsibility = inputString(input, "responsibility");
  const childId = useMessageStore((s) =>
    sessionId && call_id
      ? s.bySession.get(sessionId)?.subagentBindings?.[call_id]
      : undefined,
  );
  const child = useSessionStore((s) =>
    childId ? s.sessions.find((session) => session.id === childId) : undefined,
  );
  const text = childStatusWord(child, status);
  return (
    <SubagentInlineLine
      label={agent}
      secondary={responsibility}
      statusText={text}
      failed={status === "failed" || text === "error"}
      childId={childId}
      testId="subagent-launch-line"
    />
  );
}
