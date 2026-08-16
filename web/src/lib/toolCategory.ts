export type ProcessToolBucket = "bash" | "edit" | "tool";

/** Classify a tool call for ProcessGroup header bucketing. */
export function processToolBucket(name: string): ProcessToolBucket | null {
  if (name === "bash") return "bash";
  if (name === "edit") return "edit";
  if (name === "wait_shell" || name === "kill_shell") return null;
  return "tool";
}

export function isInlineTool(name: string): boolean {
  return name === "wait_shell" || name === "kill_shell";
}
