import { turnMapIsBusy, useTurnStore } from "../stores/turnStore";

/**
 * Keep the desktop host in step with the workspace: while a session is running
 * or cancelling, closing the window asks before it kills the turn. Only the
 * main window runs this app (popouts load a separate document), and browser
 * mode has no bridge — there the tab belongs to the browser.
 */
export function startSessionActivityBridge(): () => void {
  const push = (active: boolean) => {
    void window.litecode?.setSessionsActive?.(active)?.catch(() => undefined);
  };
  let last = turnMapIsBusy(useTurnStore.getState().byId);
  push(last);
  return useTurnStore.subscribe((state) => {
    const active = turnMapIsBusy(state.byId);
    if (active === last) return;
    last = active;
    push(active);
  });
}
