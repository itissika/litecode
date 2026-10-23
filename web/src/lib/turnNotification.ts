/**
 * Turn-end desktop toasts.
 *
 * The renderer decides *whether* to notify — only when this window cannot be
 * seen — and what the toast says. Main only shows it and brings the workbench
 * forward on click. Subagent turns never reach here: forwarded child events are
 * dropped by `shouldIgnoreForwardedSubagentEvent` before the frame switch.
 */
import type { TurnFinished } from "../api/types";

const MAX_TITLE = 80;
const MAX_BODY = 160;
const SEEN_LIMIT = 50;

const seenTurnIds: string[] = [];

function collapse(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** First non-empty line, whitespace-collapsed. */
export function firstLine(text: string | null | undefined): string {
  if (!text) return "";
  for (const line of text.split(/\r?\n/)) {
    const value = collapse(line);
    if (value) return value;
  }
  return "";
}

/** Body line for a finished turn. */
export function turnNotificationBody(finished: TurnFinished): string {
  switch (finished.reason) {
    case "completed":
      return firstLine(finished.final_text) || "Completed";
    case "error":
      return firstLine(finished.error?.message) || "Failed";
    case "max_steps":
      return "Stopped at the step limit";
    case "cancelled":
      return "Stopped";
    default:
      return "Finished";
  }
}

/** False while the user can see this window; minimized/hidden counts as unseen. */
export function turnWindowUnseen(): boolean {
  if (typeof document === "undefined") return false;
  return document.visibilityState === "hidden" || !document.hasFocus();
}

function alreadyNotified(turnId: string): boolean {
  if (!turnId) return false;
  if (seenTurnIds.includes(turnId)) return true;
  seenTurnIds.push(turnId);
  if (seenTurnIds.length > SEEN_LIMIT) seenTurnIds.shift();
  return false;
}

/** Test hook: forget which turns were announced. */
export function resetTurnNotifications(): void {
  seenTurnIds.length = 0;
}

export function maybeNotifyTurnEnd(finished: TurnFinished, sessionTitle: string): void {
  if (typeof window === "undefined") return;
  const bridge = window.litecode;
  if (!bridge?.notifyTurnEnd) return;
  if (!turnWindowUnseen()) return;
  if (alreadyNotified(finished.turn_id)) return;
  const title = collapse(sessionTitle).slice(0, MAX_TITLE) || "Litecode";
  const body = collapse(turnNotificationBody(finished)).slice(0, MAX_BODY);
  bridge.notifyTurnEnd({
    sessionId: finished.session_id,
    turnId: finished.turn_id,
    title,
    body,
  });
}
