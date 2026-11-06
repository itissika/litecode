import type { BotInfo } from "../api/types";
import { useBotStore } from "../stores/botStore";
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
