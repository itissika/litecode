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

/**
 * The server is the source of truth. An action's HTTP response can arrive after
 * a poll already observed the finished state, so settle busy flags from the
 * snapshot instead of holding a spinner until the request returns.
 */
function settleBusy(
  busy: Record<string, AppAction>,
  snapshot: AppsSnapshot,
): Record<string, AppAction> {
  const next = { ...busy };
  for (const app of snapshot.apps) {
    const action = next[app.id];
    if (!action) continue;
    const settled =
      (action === "install" &&
        (app.install?.installed === true || app.install?.error != null)) ||
      (action === "start" && app.running) ||
      (action === "stop" && !app.running) ||
      (action === "uninstall" && app.install?.installed === false) ||
      (action === "trust" && app.trusted);
    if (settled) delete next[app.id];
  }
  return next;
}

export const useAppsStore = create<AppsStore>((set, get) => {
  let loadPromise: Promise<void> | null = null;
  let reloadRequested = false;

  const applySnapshot = (snapshot: AppsSnapshot) => {
    set((state) => ({
      snapshot,
      apps: snapshot.apps,
      busy: settleBusy(state.busy, snapshot),
      error: null,
    }));
  };

  /**
   * Polls and actions share one request. A caller arriving while a fetch is in
   * flight joins it and asks for one trailing fetch, so a slow poll can never
   * starve the state update an action needs.
   */
  const load = (): Promise<void> => {
    if (loadPromise) {
      reloadRequested = true;
      return loadPromise;
    }
    const initial = !get().snapshot;
    loadPromise = (async () => {
      do {
        reloadRequested = false;
        if (initial) set({ loading: true });
        try {
          applySnapshot(await fetchApps());
        } catch (error) {
          set({ error: messageOf(error) });
        }
      } while (reloadRequested);
      set({ loading: false });
    })().finally(() => {
      loadPromise = null;
      if (reloadRequested) {
        reloadRequested = false;
        void get().load();
      }
    });
    return loadPromise;
  };

  const run = async (id: string, action: AppAction, task: () => Promise<void>) => {
    set((state) => ({ busy: setBusy(state.busy, id, action), error: null }));
    try {
      await task();
      await load();
    } catch (error) {
      set({ error: messageOf(error) });
      throw error;
    } finally {
      set((state) =>
        state.busy[id] === action
          ? { busy: setBusy(state.busy, id, null) }
          : {},
      );
    }
  };

  return {
    snapshot: null,
    apps: [],
    loading: false,
    error: null,
    busy: {},

    load,

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
