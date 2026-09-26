/**
 * Tool-card / process-group work signal. Session/turn running is not an input.
 *
 * `function_call` completed means arguments are sealed, not that the tool ran.
 * Stay live until a matching output exists (or the call is failed/incomplete).
 */
export function isToolCallLive(opts: {
  callStatus?: string;
  hasOutput: boolean;
  outputInProgress?: boolean;
}): boolean {
  if (opts.callStatus === "failed" || opts.callStatus === "incomplete") {
    return false;
  }
  if (opts.hasOutput) return opts.outputInProgress === true;
  return true;
}
