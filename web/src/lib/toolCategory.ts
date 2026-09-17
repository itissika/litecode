export type ProcessToolBucket = "bash" | "edit" | "tool";

/** Classify a tool call for ProcessGroup header bucketing. */
export function processToolBucket(name: string): ProcessToolBucket | null {
  if (name === "bash") return "bash";
  if (name === "edit") return "edit";
  if (name === "wait_shell" || name === "kill_shell") return null;
  // Session-mount capsules: a single-line row carries no card, so it must not
  // inflate the process-group header either.
  if (name === "todo" || name === "plan") return null;
  if (
    name === "subagent_launch" ||
    name === "subagent_wait" ||
    name === "subagent_stop" ||
    name === "subagent_send" ||
    name === "subagent_list"
  )
    return null;
  return "tool";
}

export function isInlineTool(name: string): boolean {
  return (
    name === "subagent_launch" ||
    name === "wait_shell" ||
    name === "kill_shell" ||
    name === "subagent_wait" ||
    name === "subagent_stop" ||
    name === "subagent_send" ||
    name === "subagent_list" ||
    name === "todo" ||
    name === "plan"
  );
}

/**
 * Inline-row routing for a tool node.
 *
 * `isInlineTool` is name-only; `bash` is the one call whose card-vs-row choice
 * depends on its RESULT (a `run_in_background` / timed-out command seals as a
 * background job), so the caller passes that verdict in. Foreground bash keeps
 * its rich card.
 */
export function isInlineCall(name: string, backgroundBash = false): boolean {
  return isInlineTool(name) || (name === "bash" && backgroundBash);
}
