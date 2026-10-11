import { getEnginesDetail, refreshRetrieval } from "../../../../api/workspace";

/**
 * App-wide follow-up to an embed model install (slim installer).
 *
 * The download runs in the Electron host and can be started from the Engines
 * card, the Start button, or Open Remote, and the Engines view may be closed
 * by the time it finishes. Whoever started it, once the model is on disk a
 * retrieval engine that failed for lack of it is restarted, and every open
 * Engines view is told to refetch so no stale "model missing" state lingers.
 */

export const ENGINES_CHANGED_EVENT = "litecode:engines-changed";

const COMPONENT = "embed-model";
const SETTLE_POLL_MS = 1000;
const SETTLE_MAX_POLLS = 10;

let inflight: Promise<void> | null = null;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Restart retrieval if it is wanted but unavailable, then notify Engines views. Deduped. */
export function restartRetrievalAfterEmbedInstall(
  opts: { pollMs?: number } = {},
): Promise<void> {
  if (inflight) return inflight;
  const pollMs = opts.pollMs ?? SETTLE_POLL_MS;
  inflight = (async () => {
    try {
      let detail = await getEnginesDetail();
      // A warmup that began before the model landed fails within moments; let
      // it settle so it is not left Failed. A real warmup keeps "warming" and
      // needs no help.
      for (let i = 0; i < SETTLE_MAX_POLLS && detail.retrieval.usable === "warming"; i++) {
        await sleep(pollMs);
        detail = await getEnginesDetail();
      }
      // "unavailable" = desired but Failed; request_refresh reconciles it.
      if (detail.retrieval.usable === "unavailable") await refreshRetrieval();
    } catch (error) {
      console.warn("retrieval restart after embed install failed", error);
    } finally {
      if (typeof window !== "undefined") window.dispatchEvent(new Event(ENGINES_CHANGED_EVENT));
    }
  })().finally(() => {
    inflight = null;
  });
  return inflight;
}

/** Listen for host "embed model installed" events for the life of the app. */
export function installEmbedInstallWatcher(): () => void {
  if (typeof window === "undefined") return () => undefined;
  const bridge = window.litecode;
  if (!bridge?.onComponentProgress || bridge.getSessionMode?.() === "remote") return () => undefined;
  const stop = bridge.onComponentProgress((event) => {
    if (event.id === COMPONENT && event.phase === "done") void restartRetrievalAfterEmbedInstall();
  });
  return () => stop?.();
}
