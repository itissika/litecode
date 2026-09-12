import type { ToolViewProps } from "./registry";

/**
 * Single-line inline view for `subagent_launch`: agent name + status word.
 *
 * The launched subagent's own transcript is intentionally NOT rendered in the
 * parent transcript — it lives in the dock's Workers panel, where the bound
 * child session can be expanded. Here the call is just a one-line status row,
 * same shape as the wait/stop rows.
 */
export function SubagentLaunchToolView({ input, status }: ToolViewProps) {
  const agent =
    input &&
    typeof input === "object" &&
    !Array.isArray(input) &&
    typeof (input as Record<string, unknown>).agent === "string"
      ? ((input as Record<string, unknown>).agent as string)
      : "subagent";
  const text =
    status === "failed"
      ? "failed"
      : status === "running"
        ? "running"
        : "completed";
  return (
    <span
      className="flex min-w-0 items-center gap-1.5"
      data-testid="subagent-launch-line"
    >
      <span className="shrink-0 font-mono text-(--_dk-text-primary)">
        {agent}
      </span>
      <span
        className={`min-w-0 truncate ${
          status === "failed" ? "text-(--_dk-red-500)" : "text-(--_dk-text-muted)"
        }`}
      >
        {text}
      </span>
    </span>
  );
}
