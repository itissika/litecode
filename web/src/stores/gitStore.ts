import { create } from "zustand";

import {
  gitCommit,
  gitLog,
  gitPull,
  gitPush,
  gitRestore,
  gitStage,
  gitStatus,
  gitUnstage,
  type GitCommitInfo,
  type GitFile,
  type GitStatus,
} from "../api/workspace";
import { attachSiblingStores } from "./connectionStore";
import { useToastStore } from "./toastStore";

export type GitSection = "staged" | "changes";

export function gitRowId(section: GitSection, path: string): string {
  return `${section}:${path}`;
}

export function parseGitRowId(id: string): { section: GitSection; path: string } | null {
  const split = id.indexOf(":");
  if (split <= 0) return null;
  const section = id.slice(0, split);
  if (section !== "staged" && section !== "changes") return null;
  return { section, path: id.slice(split + 1) };
}

/** `.git` internals git status/log may touch; they must not retrigger SCM refresh. */
export function isGitMetaPath(path: string): boolean {
  const n = path.replace(/\\/g, "/").replace(/^\.\//, "");
  return n === ".git" || n.startsWith(".git/") || n.includes("/.git/");
}

export function watchPathsAffectGitWorktree(paths: string[] | undefined): boolean {
  if (!paths || paths.length === 0) return true;
  return paths.some((p) => !isGitMetaPath(p));
}

const emptyStatus = (): GitStatus => ({
  is_repo: false,
  branch: null,
  upstream_ahead: 0,
  upstream_behind: 0,
  staged: [],
  changes: [],
});

interface GitStore {
  status: GitStatus;
  commits: GitCommitInfo[];
  message: string;
  selected: Set<string>;
  anchorId: string | null;
  loading: boolean;
  mutating: boolean;
  error: string | null;

  setMessage: (message: string) => void;
  select: (id: string, opts?: { additive?: boolean; range?: boolean; visible?: string[] }) => void;
  clearSelection: () => void;
  refresh: (opts?: { silent?: boolean }) => Promise<void>;
  scheduleRefresh: (paths?: string[]) => void;
  stagePaths: (paths: string[]) => Promise<void>;
  unstagePaths: (paths: string[]) => Promise<void>;
  restorePaths: (paths: string[]) => Promise<void>;
  commit: () => Promise<void>;
  pull: () => Promise<void>;
  push: () => Promise<void>;
}

let refreshTimer: ReturnType<typeof setTimeout> | null = null;
let refreshInFlight = false;
let refreshQueued = false;
const REFRESH_DEBOUNCE_MS = 400;

function toastError(err: unknown) {
  const message = err instanceof Error ? err.message : String(err);
  useToastStore.getState().showToast(message, "error", 8000, "git");
}

export function selectedPaths(selected: Set<string>, section: GitSection): string[] {
  const out: string[] = [];
  for (const id of selected) {
    const parsed = parseGitRowId(id);
    if (parsed?.section === section) out.push(parsed.path);
  }
  return out;
}

export const useGitStore = create<GitStore>((set, get) => ({
  status: emptyStatus(),
  commits: [],
  message: "",
  selected: new Set(),
  anchorId: null,
  loading: false,
  mutating: false,
  error: null,

  setMessage: (message) => set({ message }),

  select: (id, opts) => {
    if (opts?.range && get().anchorId && opts.visible) {
      const vis = opts.visible;
      const a = vis.indexOf(get().anchorId!);
      const b = vis.indexOf(id);
      if (a >= 0 && b >= 0) {
        const lo = Math.min(a, b);
        const hi = Math.max(a, b);
        set({
          selected: new Set(vis.slice(lo, hi + 1)),
        });
        return;
      }
    }
    if (opts?.additive) {
      const next = new Set(get().selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      set({ selected: next, anchorId: id });
      return;
    }
    set({ selected: new Set([id]), anchorId: id });
  },

  clearSelection: () => set({ selected: new Set(), anchorId: null }),

  refresh: async (opts) => {
    if (refreshInFlight) {
      refreshQueued = true;
      return;
    }
    refreshInFlight = true;
    const silent = opts?.silent ?? true;
    if (!silent) set({ loading: true, error: null });
    try {
      const [status, log] = await Promise.all([gitStatus(), gitLog(50)]);
      set({
        status,
        commits: log.commits,
        loading: false,
        error: null,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      set({ loading: false, error: silent ? get().error : message });
    } finally {
      refreshInFlight = false;
      if (refreshQueued) {
        refreshQueued = false;
        void get().refresh({ silent: true });
      }
    }
  },

  scheduleRefresh: (paths) => {
    if (!watchPathsAffectGitWorktree(paths)) return;
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => {
      refreshTimer = null;
      void get().refresh({ silent: true });
    }, REFRESH_DEBOUNCE_MS);
  },

  stagePaths: async (paths) => {
    if (paths.length === 0) return;
    set({ mutating: true });
    try {
      await gitStage(paths);
      get().clearSelection();
      await get().refresh();
    } catch (err) {
      toastError(err);
    } finally {
      set({ mutating: false });
    }
  },

  unstagePaths: async (paths) => {
    if (paths.length === 0) return;
    set({ mutating: true });
    try {
      await gitUnstage(paths);
      get().clearSelection();
      await get().refresh();
    } catch (err) {
      toastError(err);
    } finally {
      set({ mutating: false });
    }
  },

  restorePaths: async (paths) => {
    if (paths.length === 0) return;
    set({ mutating: true });
    try {
      await gitRestore(paths);
      get().clearSelection();
      await get().refresh();
    } catch (err) {
      toastError(err);
    } finally {
      set({ mutating: false });
    }
  },

  commit: async () => {
    const message = get().message.trim();
    if (!message) {
      useToastStore.getState().showToast("Commit message is required", "error", 4000, "git");
      return;
    }
    set({ mutating: true });
    try {
      await gitCommit(message);
      set({ message: "" });
      await get().refresh();
    } catch (err) {
      toastError(err);
    } finally {
      set({ mutating: false });
    }
  },

  pull: async () => {
    set({ mutating: true });
    try {
      await gitPull();
      await get().refresh();
    } catch (err) {
      toastError(err);
    } finally {
      set({ mutating: false });
    }
  },

  push: async () => {
    set({ mutating: true });
    try {
      await gitPush();
      await get().refresh();
    } catch (err) {
      toastError(err);
    } finally {
      set({ mutating: false });
    }
  },
}));

attachSiblingStores({ git: useGitStore });

export type { GitFile };
