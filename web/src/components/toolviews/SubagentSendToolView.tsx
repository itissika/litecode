import type { ToolViewProps } from "./registry";

/**
 * Auxiliary subagent_send view: show which child_session_id was resumed.
 */
export function SubagentSendToolView({ input, output, status }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const childId = typeof obj.id === "string" ? obj.id : undefined;
  if (status === "failed") {
    return (
      <div className="font-mono text-dk-sm text-(--_dk-red-500)">send failed</div>
    );
  }
  return (
    <div className="font-mono text-dk-sm text-(--_dk-text-muted)">
      {childId ? `sent to ${childId}` : output ? "sent" : "sending…"}
    </div>
  );
}
