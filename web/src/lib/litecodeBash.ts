/** Agent bash live-log RPCs (tee tail + kill). Not the human PTY API. */

import { useConnectionStore } from "../stores/connectionStore";
import { useBashStore } from "../stores/bashStore";
import { useToastStore } from "../stores/toastStore";
import type { BashTailResult } from "../api/types";

/** `bash/kill` answer: the state after the signal, not the signal itself. */
export interface BashKillResult {
  /** The process outlived the backend's reap wait. */
  alive?: boolean;
  exit_code?: number | null;
  /** The job was already gone: a stale card the panel can drop. */
  gone?: boolean;
}

export async function bashTail(bashId: string): Promise<BashTailResult> {
  return useConnectionStore.getState().sendRpc<BashTailResult>("bash/tail", {
    bash_id: bashId,
  });
}

export async function bashKill(bashId: string): Promise<BashKillResult> {
  const result = await useConnectionStore
    .getState()
    .sendRpc<BashKillResult>("bash/kill", { bash_id: bashId });
  return result ?? {};
}

/**
 * Kill from a UI surface (terminal panel, bash card, inline row).
 *
 * The kill must not fail silently: a job the backend no longer knows is dropped
 * from the panel instead of looking like a live terminal, and a request that
 * never landed is reported. A job that is still alive stays on screen — that is
 * the truth, and the next click can try again.
 */
export async function killBashJob(
  sessionId: string | undefined,
  bashId: string,
): Promise<void> {
  try {
    const result = await bashKill(bashId);
    if (result.gone && sessionId) {
      useBashStore.getState().forgetJob(sessionId, bashId);
    }
  } catch (error) {
    useToastStore
      .getState()
      .showToast(error instanceof Error ? error.message : String(error), "error");
  }
}
