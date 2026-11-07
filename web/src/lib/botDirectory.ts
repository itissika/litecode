import type { BotInfo, BotPosition } from "../api/types";
import { sessionTitle } from "./sessionDisplay";

/**
 * Pure bot-directory projection. Store-free on purpose: connectionStore and
 * other low-level modules can name a bot session without importing the store.
 */
export function botForSession(
  bots: readonly BotInfo[],
  sessionId: string | undefined,
): BotInfo | undefined {
  if (!sessionId) return undefined;
  return bots.find((bot) => bot.session_id === sessionId);
}

/** Placement. The wire has one position field; no compatibility fallback. */
export function botPosition(bot: BotInfo | undefined): BotPosition {
  return bot?.position ?? { kind: "none" };
}

export function botParentId(bot: BotInfo | undefined): string | null {
  const position = botPosition(bot);
  return position.kind === "bot" ? position.id : null;
}

export function botFolderId(bot: BotInfo | undefined): string | null {
  const position = botPosition(bot);
  return position.kind === "folder" ? position.id : null;
}

/** Bot rows and their tab/editor/header all read this one value. */
export function sessionDisplayTitle(
  session: { title?: string | null; preview?: string | null } | undefined,
  bot: { name?: string } | null | undefined,
): string {
  return bot?.name?.trim() || sessionTitle(session);
}
