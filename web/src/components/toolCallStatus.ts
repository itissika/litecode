import { functionCallOutputText } from "../api/adapter";
import type { FunctionCallOutputItem } from "../api/types";
import type { ToolStatus } from "./ToolIcon";

function callWillNotProduceOutput(status?: string): boolean {
  return status === "failed" || status === "incomplete";
}

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
  if (callWillNotProduceOutput(opts.callStatus)) return false;
  if (opts.hasOutput) return opts.outputInProgress === true;
  return true;
}

/**
 * Process FoldCards represent a contiguous tool/reasoning segment, not a single
 * tool invocation. The group stays open until the following assistant message
 * arrives (closing the segment) or a terminal stop (failed/incomplete) occurs.
 * Streaming state is irrelevant — a live node cannot coexist with a following
 * message or terminal stop, so the product semantics reduce to two conditions.
 */
export function processGroupAutoOpen(opts: {
  followedByMessage: boolean;
  hasTerminalStop: boolean;
}): boolean {
  return !opts.followedByMessage && !opts.hasTerminalStop;
}

/** Shared tool-call status used by both the tool card and the process group.
 *
 * `streaming` here is work-live: arguments still open, or waiting for / streaming output.
 */
export function deriveToolStatus(
  output?: FunctionCallOutputItem,
  streaming?: boolean,
  callStatus?: string,
): ToolStatus {
  // A stream-error invalidation sets the FunctionCall Item status to "failed";
  // recognize it explicitly, not just the "Error:" output prefix (FE-06).
  if (callStatus === "failed") return "failed";
  const resultText = output ? functionCallOutputText(output) : undefined;
  if (output) {
    if (resultText?.startsWith("Error:")) return "failed";
    if (
      resultText?.startsWith("Warning:") ||
      resultText?.includes("\n\nWarning:")
    ) {
      return "warning";
    }
    return "ok";
  }
  if (streaming) return "running";
  return "unknown";
}
