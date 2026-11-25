type ProcessToolBucket = "bash" | "edit" | "tool";

/** Classify a tool call for ProcessGroup header bucketing. */
export function processToolBucket(name: string): ProcessToolBucket | null {
  if (name === "exec_command") return "bash";
  if (name === "edit") return "edit";
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
