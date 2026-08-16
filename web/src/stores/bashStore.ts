import { create } from "zustand";

import type { BashJob, BashJobsSnapshot, BashWait } from "../api/types";

export interface SessionBash {
  jobs: BashJob[];
  waits: BashWait[];
}

interface BashStore {
  bySession: Map<string, SessionBash>;
  applySnapshot: (sessionId: string, snap: BashJobsSnapshot) => void;
  reset: (sessionId?: string) => void;
}

export const useBashStore = create<BashStore>((set) => ({
  bySession: new Map(),

  applySnapshot: (sessionId, snap) => {
    if (!sessionId) return;
    const bySession = new Map(useBashStore.getState().bySession);
    bySession.set(sessionId, {
      jobs: snap.jobs ?? [],
      waits: snap.waits ?? [],
    });
    set({ bySession });
  },

  reset: (sessionId) => {
    if (!sessionId) {
      set({ bySession: new Map() });
      return;
    }
    const bySession = new Map(useBashStore.getState().bySession);
    bySession.delete(sessionId);
    set({ bySession });
  },
}));
