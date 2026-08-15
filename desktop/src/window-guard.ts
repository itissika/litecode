import type { WebContents } from "electron";

export type BrowserWindowLike = {
  isDestroyed(): boolean;
};

/**
 * Resolve the target window for an IPC message strictly from the sender's
 * own webContents (DESK-01). A window is only trusted when it is the
 * sender's live window; there is deliberately no global fallback, so a
 * message from an unknown, destroyed, or sub-frame webContents can never
 * drive another window.
 */
export function senderOwnedWindow(
  sender: WebContents | null,
  fromWebContents: (sender: WebContents) => BrowserWindowLike | null,
): BrowserWindowLike | null {
  if (!sender) return null;
  const win = fromWebContents(sender);
  return win && !win.isDestroyed() ? win : null;
}
