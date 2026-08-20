import { functionCallOutputText } from "../api/adapter";
import type { FunctionCallOutputItem } from "../api/types";
import type { ToolStatus } from "./ToolIcon";

/**
 * Tool cards stay live while the call seq or its output seq is in_progress.
 * Session/turn running is not an input.
 */
export function isToolCallLive(
  rowStreaming: boolean,
  outputRowStreaming = false,
): boolean {
  return rowStreaming || outputRowStreaming;
}

/**
 * Process FoldCard open/live signal — only seqs in this group.
 */
export function processGroupStreaming(opts: { hasInProgress: boolean }): boolean {
  return opts.hasInProgress;
}

/** Shared tool-call status used by both the tool card and the process group.
 *
 * `streaming` covers live argument streaming and a still-in_progress output seq.
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
