import { create } from "zustand";

import {
  fetchTree,
  type TreeEntry,
  type WorkspaceChangeKind,
} from "../api/workspace";
import { parentPath, remapPathPrefix } from "../utils/path";
import { attachSiblingStores } from "./connectionStore";

interface TreeStore {
  children: Record<string, TreeEntry[]>;
  expanded: Set<string>;
  loading: Set<string>;
  error: string | null;

  loadRoot: () => Promise<void>;
  loadChildren: (path: string) => Promise<void>;
  toggleExpand: (path: string, kind: "file" | "dir") => Promise<void>;
  invalidate: (path: string) => void;
  collapseAll: () => void;
  refreshAll: () => Promise<void>;
  expandDir: (path: string) => Promise<void>;
  /** Expand every ancestor of `path` so the file becomes visible in the tree. */
  revealPath: (path: string) => Promise<void>;
  handleWorkspaceChange: (
    paths: string[],
    kind: WorkspaceChangeKind,
  ) => Promise<void>;
}

const WORKSPACE_DEBOUNCE_MS = 200;

let workspaceDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let pendingWorkspacePaths: string[] = [];
let pendingWorkspaceKind: WorkspaceChangeKind = "modified";

function cloneSet(s: Set<string>): Set<string> {
  return new Set(s);
}

function parentsToRefresh(path: string): string[] {
  const parents: string[] = [""];
  let cur = parentPath(path);
  while (cur) {
    parents.push(cur);
    cur = parentPath(cur);
  }
  return parents;
}

function collectRefreshKeys(
  paths: string[],
  kind: WorkspaceChangeKind,
): Set<string> {
  const toRefresh = new Set<string>();
  for (const p of paths) {
    for (const parent of parentsToRefresh(p)) {
      toRefresh.add(parent);
    }
    if (kind !== "deleted") {
      toRefresh.add(parentPath(p));
    }
  }
  return toRefresh;
}

export const useTreeStore = create<TreeStore>((set, get) => {
  async function refreshWorkspaceKeys(keys: Set<string>): Promise<void> {
    const { expanded, children } = get();
    const reloads: Promise<void>[] = [];
    const toInvalidate: string[] = [];

    for (const key of keys) {
      if (key === "" || expanded.has(key)) {
        if (children[key]) {
          reloads.push(get().loadChildren(key));
        } else if (key === "") {
          reloads.push(get().loadChildren(key));
        }
      } else {
        toInvalidate.push(key);
      }
    }

    if (toInvalidate.length > 0) {
      set((s) => {
        const next = { ...s.children };
        for (const key of toInvalidate) {
          delete next[key];
        }
        return { children: next };
      });
    }

    await Promise.all(reloads);
  }

  function flushPendingWorkspaceChanges(): void {
    workspaceDebounceTimer = null;
    if (pendingWorkspacePaths.length === 0) return;

    const paths = pendingWorkspacePaths;
    const kind = pendingWorkspaceKind;
    pendingWorkspacePaths = [];
    const keys = collectRefreshKeys(paths, kind);
    void refreshWorkspaceKeys(keys);
  }

  function queueWorkspaceChange(
    paths: string[],
    kind: WorkspaceChangeKind,
  ): void {
    pendingWorkspacePaths.push(...paths);
    pendingWorkspaceKind = kind;

    if (workspaceDebounceTimer) {
      clearTimeout(workspaceDebounceTimer);
    }
    workspaceDebounceTimer = setTimeout(
      flushPendingWorkspaceChanges,
      WORKSPACE_DEBOUNCE_MS,
    );
  }

  return {
    children: {},
    expanded: new Set<string>(),
    loading: new Set<string>(),
    error: null,

    loadRoot: async () => {
      await get().loadChildren("");
    },

    loadChildren: async (path) => {
      const { loading } = get();
      if (loading.has(path)) return;

      const nextLoading = cloneSet(loading);
      nextLoading.add(path);
      set({ loading: nextLoading, error: null });

      try {
        const entries = await fetchTree(path, 1);
        set((s) => {
          const nl = cloneSet(s.loading);
          nl.delete(path);
          return {
            children: { ...s.children, [path]: entries },
            loading: nl,
          };
        });
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        set((s) => {
          const nl = cloneSet(s.loading);
          nl.delete(path);
          return { loading: nl, error: msg };
        });
      }
    },

    toggleExpand: async (path, kind) => {
      if (kind === "file") return;

      const { expanded, children } = get();
      const next = cloneSet(expanded);

      if (next.has(path)) {
        next.delete(path);
        set({ expanded: next });
        return;
      }

      next.add(path);
      set({ expanded: next });

      if (!children[path]) {
        await get().loadChildren(path);
      }
    },

    invalidate: (path) => {
      set((s) => {
        const next = { ...s.children };
        delete next[path];
        return { children: next };
      });
    },

    collapseAll: () => {
      set({ expanded: new Set<string>() });
    },

    refreshAll: async () => {
      const { expanded } = get();
      const keys = ["", ...Array.from(expanded)];
      await Promise.all(keys.map((key) => get().loadChildren(key)));
    },

    expandDir: async (path) => {
      if (!path) return;
      const next = cloneSet(get().expanded);
      next.add(path);
      set({ expanded: next });
      if (!get().children[path]) {
        await get().loadChildren(path);
      }
    },

    revealPath: async (path) => {
      if (!path) return;
      const dirs: string[] = [];
      let cur = parentPath(path);
      while (cur) {
        dirs.unshift(cur);
        cur = parentPath(cur);
      }
      for (const dir of dirs) {
        await get().expandDir(dir);
      }
    },

    handleWorkspaceChange: async (paths, kind) => {
      if (kind === "renamed" && paths.length >= 2) {
        const [from, to] = paths;
        if (from && to) {
          set((s) => {
            const children: Record<string, TreeEntry[]> = {};
            for (const [key, entries] of Object.entries(s.children)) {
              children[remapPathPrefix(key, from, to)] = entries.map((e) => ({
                ...e,
                path: remapPathPrefix(e.path, from, to),
              }));
            }
            const expanded = new Set(
              Array.from(s.expanded).map((p) => remapPathPrefix(p, from, to)),
            );
            const loading = new Set(
              Array.from(s.loading).map((p) => remapPathPrefix(p, from, to)),
            );
            return { children, expanded, loading };
          });
        }
      }
      queueWorkspaceChange(paths, kind);
    },

  };
});

attachSiblingStores({ tree: useTreeStore });
