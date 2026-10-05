import type { LlmReconnectNotice } from "../api/types";

/** `500` → `0.5s`, `1000` and above → whole seconds. */
export function formatReconnectDelay(delayMs: number): string {
  if (delayMs >= 1000) return `${Math.round(delayMs / 1000)}s`;
  return `${Math.round(delayMs / 100) / 10}s`;
}

export function formatLlmReconnect(notice: LlmReconnectNotice): string {
  const count = `${notice.attempt}/${notice.max_attempts}`;
  if (notice.phase === "waiting") {
    return `Waiting ${formatReconnectDelay(notice.delay_ms ?? 0)} before reconnecting (${count})`;
  }
  if (notice.phase === "connecting") {
    return `Reconnecting (${count})`;
  }
  if (notice.phase === "failed") {
    if (notice.attempt <= 1) return "Reconnection failed";
    return `Reconnection failed (${count})`;
  }
  return "";
}

/** Keep a failed bubble across turn end; drop a clear. */
export function reconnectAfterFinish(
  retryable: boolean,
  current: LlmReconnectNotice | null,
  snapshot: LlmReconnectNotice | null | undefined,
): LlmReconnectNotice | null {
  if (!retryable) return null;
  if (current?.phase === "failed") return current;
  if (snapshot && snapshot.phase !== "cleared") return snapshot;
  return null;
}
