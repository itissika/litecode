import { create } from "zustand";

import {
  fetchApps,
  installApp,
  startApp,
  stopApp,
  trustApp,
  uninstallApp,
  type AppsSnapshot,
  type AppView,
} from "../api/apps";

export type AppAction = "install" | "start" | "stop" | "uninstall" | "trust";

interface AppsStore {
  snapshot: AppsSnapshot | null;
  apps: AppView[];
  loading: boolean;
  error: string | null;
  busy: Record<string, AppAction>;
  load: () => Promise<void>;
  install: (id: string) => Promise<void>;
  start: (id: string) => Promise<void>;
  stop: (id: string) => Promise<void>;
  uninstall: (id: string) => Promise<void>;
  trust: (id: string, definitionHash: string) => Promise<void>;
  clearError: () => void;
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function setBusy(
  busy: Record<string, AppAction>,
  id: string,
  action: AppAction | null,
): Record<string, AppAction> {
  const next = { ...busy };
  if (action) next[id] = action;
  else delete next[id];
  return next;
}

export const useAppsStore = create<AppsStore>((set, get) => {
  let loadSequence = 0;

  const run = async (id: string, action: AppAction, task: () => Promise<void>) => {
    set((state) => ({ busy: setBusy(state.busy, id, action), error: null }));
    try {
      await task();
      await get().load();
    } catch (error) {
      set({ error: messageOf(error) });
      throw error;
    } finally {
      set((state) => ({ busy: setBusy(state.busy, id, null) }));
    }
  };

  return {
    snapshot: null,
    apps: [],
    loading: false,
    error: null,
    busy: {},

    load: async () => {
      // Polls and actions can overlap. The newest request wins, so a slower
      // stale poll cannot overwrite the state an action just fetched.
      const sequence = ++loadSequence;
      set({ loading: true });
      try {
        const snapshot = await fetchApps();
        if (sequence === loadSequence) {
          set({
            snapshot,
            apps: snapshot.apps,
            error: null,
          });
        }
      } catch (error) {
        if (sequence === loadSequence) set({ error: messageOf(error) });
      } finally {
        if (sequence === loadSequence) set({ loading: false });
      }
    },

    install: async (id) => {
      await run(id, "install", () => installApp(id));
    },
    start: async (id) => {
      await run(id, "start", () => startApp(id));
    },
    stop: async (id) => {
      await run(id, "stop", () => stopApp(id));
    },
    uninstall: async (id) => {
      await run(id, "uninstall", () => uninstallApp(id));
    },
    trust: async (id, definitionHash) => {
      await run(id, "trust", () => trustApp(id, definitionHash));
    },
    clearError: () => set({ error: null }),
  };
});
