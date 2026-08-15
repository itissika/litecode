import { functionCallOutputText } from "../api/adapter";
import type { FunctionCallOutputItem } from "../api/types";
import type { ToolStatus } from "./ToolIcon";

/**
 * Tool cards stay "live" while args stream, while the output row streams, or
 * while the turn is still active after an early function_call seal (no output yet).
 */
export function isToolCallLive(
  hasOutput: boolean,
  rowStreaming: boolean,
  turnActive: boolean,
  outputRowStreaming = false,
): boolean {
  return rowStreaming || outputRowStreaming || (turnActive && !hasOutput);
}

/**
 * Process FoldCard open/live signal — product rule is text-gated only.
 *
 * Stay open while the turn is running and no assistant text block has appeared
 * after this process group. Collapse once text follows (phase done) or the turn ends.
 * Child tool/reasoning streaming must NOT drive this (avoids close→open flicker
 * between tool batches).
 */
export function processGroupStreaming(opts: {
  hasTextAfter: boolean;
  turnActive: boolean;
}): boolean {
  if (opts.hasTextAfter) return false;
  return opts.turnActive;
}

/** Shared tool-call status used by both the tool card and the process group.
 *
 * `streaming` covers live argument streaming and the post-seal window while
 * the turn is still active but `function_call_output` has not arrived yet.
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
