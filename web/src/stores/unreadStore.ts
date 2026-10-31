import { create } from "zustand";

import { panelVisible } from "../dockview/workbench/queries";

/**
 * Frontend-only "finished while you were not looking" marks for the sidebar.
 * Nothing is persisted: a reload starts with every session read.
 */
interface UnreadStore {
  /** Root sessions that settled to idle while their transcript was hidden. */
  unread: Record<string, true>;
  mark: (sessionId: string) => void;
  /** The panel became active; the mark has been seen. */
  consume: (sessionId: string) => void;
}

export const useUnreadStore = create<UnreadStore>((set) => ({
  unread: {},

  mark: (sessionId) => {
    set((state) =>
      state.unread[sessionId]
        ? state
        : { unread: { ...state.unread, [sessionId]: true } },
    );
  },

  consume: (sessionId) => {
    set((state) => {
      if (!state.unread[sessionId]) return state;
      const unread = { ...state.unread };
      delete unread[sessionId];
      return { unread };
    });
  },
}));

/**
 * A root session settled to idle. Only a transcript the user could not see
 * earns a mark; focusing that panel consumes it (see the dock panel bootstrap).
 */
export function noteSessionFinished(sessionId: string): void {
  // A root is normally hosted by `agent-*`, but it can be classified before
  // the session list arrives and land in the read-only `subagent-*` host.
  if (
    panelVisible(`agent-${sessionId}`) ||
    panelVisible(`subagent-${sessionId}`)
  ) {
    return;
  }
  useUnreadStore.getState().mark(sessionId);
}
