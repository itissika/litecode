/** Bot-owned agent profiles use this reserved id prefix (mirrors the backend). */
export const BOT_PROFILE_PREFIX = "bot_";

/** A session whose `agent_id` is a bot-owned profile cannot switch agents. */
export function isBotProfileId(id: string | null | undefined): boolean {
  return typeof id === "string" && id.startsWith(BOT_PROFILE_PREFIX);
}
