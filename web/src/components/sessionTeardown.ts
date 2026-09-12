import { getDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { useNotificationStore } from "../stores/notificationStore";
import { useTurnStore } from "../stores/turnStore";
import { clearFoldCardOpen } from "./foldCardState";
import { isSubagentRosterHeld } from "./subagentRosterHolds";

export interface TeardownScope {
  /**
   * Also drop the local projection (message + turn slices). `false` keeps them
   * so the next expand can catch up INCREMENTALLY instead of cold-starting.
   */
  dropProjection: boolean;
}

/** The session's own dock tab (`agent-<id>`) is open and owns its stream. */
function ownedByDockTab(sessionId: string): boolean {
  return !!getDockviewApi()?.getPanel(`agent-${sessionId}`);
}

/**
 * The single teardown for every session surface (dock tab, roster card).
 *
 * `ensureSubscribe` / `unsubscribeSession` are not refcounted, so each surface
 * has to know what the others still render — the guards live in the two entry
 * points below, this core only does the work:
 *
 *   - `notificationStore.reset` + `clearFoldCardOpen` are SURFACE state: stale
 *     toasts and FoldCard open-intents must never survive into a re-opened tab.
 *   - the message/turn slices are PROJECTION state and are optional: a tab close
 *     drops them, a roster-card collapse keeps them (P6 — the child's context is
 *     worth the bounded memory, and a re-expand tops it up via the snapshot gap).
 */
export function teardownSession(sessionId: string, scope: TeardownScope): void {
  useConnectionStore.getState().unsubscribeSession(sessionId);
  useNotificationStore.getState().reset(sessionId);
  clearFoldCardOpen(sessionId);
  if (scope.dropProjection) {
    useMessageStore.getState().reset(sessionId);
    useTurnStore.getState().resetTurn(sessionId);
  }
}

/**
 * A session's own dock tab is closing.
 *
 * Skipped while an expanded dock "Workers" roster card still holds the session:
 * the child can be rendered in a tab AND in a roster card at once, and tearing
 * the subscription down here would kill the card's stream with nothing to
 * re-arm it.
 */
export function releaseSessionTab(sessionId: string): void {
  if (isSubagentRosterHeld(sessionId)) return;
  teardownSession(sessionId, { dropProjection: true });
}

/**
 * An expanded "Workers" roster card is collapsing.
 *
 * Skipped while the child still owns its `agent-<childId>` dock tab (that tab
 * manages the subscription for as long as it lives). Otherwise the subscription
 * goes, but the message/turn slices STAY (P6): re-expanding runs
 * `ensureSubscribe` and the snapshot handler appends the missing tail.
 */
export function releaseSubagentCard(sessionId: string): void {
  if (ownedByDockTab(sessionId)) return;
  teardownSession(sessionId, { dropProjection: false });
}
