import type { ToolViewProps } from "./registry";

/**
 * Auxiliary kill_shell view: show which bash_id was stopped. Human Kill lives
 * on the bash tool card only.
 */
export function KillShellToolView({ input }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const bashId = typeof obj.bash_id === "string" ? obj.bash_id : undefined;
  return (
    <div className="font-mono text-dk-sm text-(--_dk-text-muted)">
      {bashId ? `killed ${bashId}` : "killed"}
    </div>
  );
}
