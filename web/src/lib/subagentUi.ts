import type { SessionInfo } from "../api/types";
import type { ToolStatus } from "../components/ToolIcon";

export function inputString(input: unknown, key: string): string | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input))
    return undefined;
  const value = (input as Record<string, unknown>)[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

export function outputField(text: string, key: string): string | undefined {
  const match = new RegExp(`^${key}:\\s*(.+)$`, "m").exec(text);
  const value = match?.[1]?.trim();
  return value ? value : undefined;
}

export function countWaitIds(input: unknown): number | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input))
    return undefined;
  const ids = (input as Record<string, unknown>).ids;
  if (!Array.isArray(ids)) return undefined;
  const n = ids.filter((id) => typeof id === "string" && id.length > 0).length;
  return n > 0 ? n : undefined;
}

export function waitTargetCount(input: unknown): number | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input))
    return undefined;
  const count = (input as Record<string, unknown>).count;
  if (typeof count === "number" && Number.isFinite(count) && count > 0) {
    return Math.floor(count);
  }
  return countWaitIds(input);
}

/** Human-facing child status for launch/send rows. Prefers live Session over the sealed tool call. */
export function childStatusWord(
  child: SessionInfo | undefined,
  toolStatus: ToolStatus,
): string {
  if (toolStatus === "failed") return "failed";
  if (child?.status === "stopping" || child?.turn?.phase === "cancelling") {
    return "stopping";
  }
  if (
    child?.running ||
    child?.status === "running" ||
    child?.status === "running_with_subagent"
  ) {
    return "running";
  }
  if (child?.last_turn_reason) return child.last_turn_reason;
  if (child) return "idle";
  if (toolStatus === "running" || toolStatus === "unknown") return "running";
  return "accepted";
}

export function sendStatusWord(
  child: SessionInfo | undefined,
  toolStatus: ToolStatus,
  outputText?: string,
): string {
  if (toolStatus === "failed") return "failed";
  if (child) return childStatusWord(child, toolStatus);
  const reported = outputText ? outputField(outputText, "status") : undefined;
  if (reported === "running") return "running";
  if (toolStatus === "running" || toolStatus === "unknown") return "sending";
  return "sent";
}

export function waitSettledLine(outputText: string): string {
  const status = outputField(outputText, "status");
  if (status === "nothing to wait for") return "nothing to wait";
  const settled = outputField(outputText, "settled");
  const agents = [...outputText.matchAll(/^agent: (\S+)/gm)].map(
    (match) => match[1]!,
  );
  const reasons = [...outputText.matchAll(/^reason: (\S+)/gm)].map(
    (match) => match[1]!,
  );
  const n = settled ? Number(settled) : agents.length || 1;
  const parts = [`settled ${n}`];
  if (agents.length === 1) parts.push(agents[0]!);
  if (reasons.length === 1) parts.push(reasons[0]!);
  const skipped = outputField(outputText, "skipped");
  if (skipped && Number(skipped) > 0) parts.push(`skipped ${skipped}`);
  return parts.join(" · ");
}

export function stopStatusWord(
  outputText: string | undefined,
  toolStatus: ToolStatus,
): string {
  if (toolStatus === "failed") return "failed";
  if (!outputText) return "stopping";
  const status = outputField(outputText, "status");
  const reason = outputField(outputText, "reason");
  if (status === "already ended") {
    return reason ? `already ended · ${reason}` : "already ended";
  }
  if (status === "idle") return "idle";
  if (status === "stop_requested") return "stop requested";
  return "stop requested";
}

export function listStatusWord(
  outputText: string | undefined,
  toolStatus: ToolStatus,
): string {
  if (toolStatus === "failed") return "failed";
  if (!outputText) return "listing…";
  const match = /^sessions: (\d+)/m.exec(outputText);
  if (!match) return "listed";
  const n = Number(match[1]);
  if (n === 0) return "none";
  return n === 1 ? "1 session" : `${n} sessions`;
}

export function truncateLine(text: string, max = 48): string {
  const flat = text.replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, max - 1)}…`;
}
