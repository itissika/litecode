/**
 * Whether closing the workbench window has to ask first. A running turn dies
 * with the window — the sidecar stops with the app — so an accidental close
 * throws away work the person can still see happening.
 */
export type CloseState = {
  /** The renderer reports at least one session running or cancelling. */
  sessionsActive: boolean;
  /** The person already chose "close anyway" for this window. */
  confirmed: boolean;
  /** A confirmation dialog is up; a second close must not stack another. */
  promptOpen: boolean;
  /** The app is already quitting and cleaning up. */
  quitting: boolean;
};

export type CloseDecision = "close" | "prompt" | "wait";

export function closeDecision(state: CloseState): CloseDecision {
  if (!state.sessionsActive || state.confirmed || state.quitting) return "close";
  return state.promptOpen ? "wait" : "prompt";
}
