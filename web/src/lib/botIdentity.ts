import type { BotInfo, SessionBotIdentity } from "../api/types";
import { useBotStore } from "../stores/botStore";
import { useSessionStore } from "../stores/sessionStore";
import { botForSession } from "./botDirectory";

export {
  botFolderId,
  botForSession,
  botParentId,
  botPosition,
  sessionDisplayTitle,
} from "./botDirectory";

/** React binding for the one bot-directory identity projection. */
export function useBotForSession(sessionId: string | undefined): BotInfo | undefined {
  return useBotStore((state) => botForSession(state.bots, sessionId));
}

/**
 * Session identity consumption. The server's `session.bot` field is primary;
 * the directory lookup is only a compatibility fallback for an older server or
 * an optimistic row that has not seen `session/list` yet.
 */
export function useSessionBotIdentity(
  sessionId: string | undefined,
): SessionBotIdentity | BotInfo | undefined {
  const fromSession = useSessionStore((state) =>
    sessionId
      ? state.sessions.find((session) => session.id === sessionId)?.bot
      : undefined,
  );
  const fromDirectory = useBotForSession(sessionId);
  // `null` is an explicit "ordinary/no-bot session"; only `undefined`
  // (missing field, optimistic row, or older server) may fall back to the
  // directory.
  if (fromSession === null) return undefined;
  return fromSession ?? fromDirectory;
}
