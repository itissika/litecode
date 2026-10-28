import { getWindows, subscribeWindows } from "./windows";

/**
 * The workbench window the user last pointed at or typed in.
 *
 * A click inside a popout is the clearest statement of "I am working here".
 * Commands often run a tick later (an awaited load, an LSP reply), so they
 * cannot read `document.hasFocus()` at open time; they read this instead.
 * The main window is the fallback, which keeps opens from a main-window
 * click in the main window even while a popout sits in front.
 */
let lastActive: Window | null = null;

export function noteUserWindow(win: Window): void {
  lastActive = win;
}

export function resetActivityForTests(): void {
  lastActive = null;
}

/** Dock id of the window the user last used; null means the main window. */
export function lastUserDockId(): string | null {
  const win = lastActive;
  if (!win || win.closed) return null;
  for (const entry of getWindows()) {
    if (entry.window === win) return entry.dockId;
  }
  return null;
}

/**
 * One listener per registered window. Capture, so a panel that stops
 * propagation on its own pointerdown cannot hide the interaction.
 */
export function bindWindowActivity(): () => void {
  return subscribeWindows((entry) => {
    const doc = entry.window.document;
    const note = () => noteUserWindow(entry.window);
    doc.addEventListener("pointerdown", note, true);
    doc.addEventListener("keydown", note, true);
    entry.window.addEventListener("focus", note);
    return () => {
      doc.removeEventListener("pointerdown", note, true);
      doc.removeEventListener("keydown", note, true);
      entry.window.removeEventListener("focus", note);
    };
  });
}
