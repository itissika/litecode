import type { ToolViewProps } from "./registry";

/**
 * Auxiliary subagent_stop view: show which child_session_id was stopped.
 */
export function SubagentStopToolView({ input }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const childId = typeof obj.id === "string" ? obj.id : undefined;
  return (
    <div className="font-mono text-dk-sm text-(--_dk-text-muted)">
      {childId ? `stopped ${childId}` : "stopped"}
    </div>
  );
}
