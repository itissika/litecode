import { create } from "zustand";

import type {
  SubagentJob,
  SubagentJobsSnapshot,
  SubagentWait,
} from "../api/types";

export interface SessionSubagent {
  jobs: SubagentJob[];
  waits: SubagentWait[];
}

interface SubagentStore {
  bySession: Map<string, SessionSubagent>;
  applySnapshot: (sessionId: string, snap: SubagentJobsSnapshot) => void;
  reset: (sessionId?: string) => void;
}

export const useSubagentStore = create<SubagentStore>((set) => ({
  bySession: new Map(),

  applySnapshot: (sessionId, snap) => {
    if (!sessionId) return;
    const bySession = new Map(useSubagentStore.getState().bySession);
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
    const bySession = new Map(useSubagentStore.getState().bySession);
    bySession.delete(sessionId);
    set({ bySession });
  },
}));
