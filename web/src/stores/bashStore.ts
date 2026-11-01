import { create } from "zustand";

import type { BashJob, BashJobsSnapshot, BashWait } from "../api/types";

interface SessionBash {
  jobs: BashJob[];
  waits: BashWait[];
}

interface BashStore {
  bySession: Map<string, SessionBash>;
  applySnapshot: (sessionId: string, snap: BashJobsSnapshot) => void;
  /** Drop one job row on its own (a card the backend reports as already gone). */
  forgetJob: (sessionId: string, jobId: string) => void;
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

  forgetJob: (sessionId, jobId) => {
    const current = useBashStore.getState().bySession.get(sessionId);
    if (!current || !current.jobs.some((job) => job.id === jobId)) return;
    const bySession = new Map(useBashStore.getState().bySession);
    bySession.set(sessionId, {
      jobs: current.jobs.filter((job) => job.id !== jobId),
      waits: current.waits,
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
