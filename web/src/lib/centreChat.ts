import type { DockviewApi } from "dockview-react";

import { useSessionStore } from "../stores/sessionStore";

/**
 * When the restored centre grid contains no panels at all, open a chat.
 *
 * Any persisted centre panel — an editor, an agent, a subagent — is the
 * user's layout. Edge rails (Explorer, Sessions, Terminal) are not the
 * centre and do not count. Ordinary layout, independent of whether a
 * provider key exists. Two signals have to arrive first: the dockview
 * layout must have finished restoring (a panel added before `fromJSON`
 * settles is wiped), and `server/hello` must have reached us (`session/new`
 * needs the socket). Once per page load. Empty sessions are collected by
 * session GC.
 */

let api: DockviewApi | null = null;
let transportReady = false;
/** Already decided for this page load, whether or not a panel was created. */
let considered = false;

/** True when the centre grid already holds any panel. Edge rails do not count. */
function centreHasPanel(dockview: DockviewApi): boolean {
  return dockview.panels.some((panel) => panel.api.location.type === "grid");
}

function ensureCentreChat(): void {
  if (!transportReady || !api || considered) return;
  considered = true;
  if (centreHasPanel(api)) return;
  useSessionStore.getState().newSession();
}

/** The dockview layout finished restoring (or was built from scratch). */
export function noteLayoutSettled(next: DockviewApi): void {
  api = next;
  ensureCentreChat();
}

/**
 * `server/hello` arrived, so the socket can take `session/new`.
 * Called before the settings summary fetch: a failed summary must not
 * leave the centre empty.
 */
export function noteTransportReady(): void {
  transportReady = true;
  ensureCentreChat();
}

/** Test hook: forget the signals this page load already saw. */
export function resetCentreChatForTests(): void {
  api = null;
  transportReady = false;
  considered = false;
}
