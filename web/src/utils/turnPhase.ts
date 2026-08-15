import type { TurnPhase } from "../api/types";

export function formatTurnPhase(phase: TurnPhase): string {
  if (phase === "idle") return "Idle";
  if (phase === "starting") return "Starting";
  if (phase === "compacting") return "Compacting";
  if (phase === "calling_llm") return "Calling LLM";
  if (phase === "streaming") return "Streaming";
  if (phase === "executing_tools") return "Executing tools";
  if (phase === "cancelling") return "Cancelling";
  if (phase === "finalizing") return "Finalizing";
  if (typeof phase === "object" && phase !== null && "awaiting_permission" in phase) {
    return `Awaiting permission: ${phase.awaiting_permission.tool}`;
  }
  if (typeof phase === "object" && phase !== null && "failed" in phase) {
    return `Failed (${phase.failed.code})`;
  }
  return "Unknown";
}

export function turnPhaseTone(
  phase: TurnPhase,
): "neutral" | "active" | "warning" | "danger" {
  if (phase === "cancelling") return "warning";
  if (typeof phase === "object" && phase !== null && "failed" in phase) return "danger";
  if (typeof phase === "object" && phase !== null && "awaiting_permission" in phase) {
    return "warning";
  }
  if (phase === "idle") return "neutral";
  return "active";
}
