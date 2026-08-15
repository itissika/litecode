import { create } from "zustand";

import { remapPathPrefix } from "../utils/path";

export type ExplorerClipboard = {
  mode: "cut" | "copy";
  paths: string[];
};

export type InlineEdit =
  | { kind: "rename"; path: string }
  | { kind: "newFile"; parent: string }
  | { kind: "newFolder"; parent: string };

export type ExplorerMenu = {
  x: number;
  y: number;
  /** Workspace path, or `null` for blank/root area. */
  path: string | null;
};

interface ExplorerStore {
  selected: Set<string>;
  anchorPath: string | null;
  focusPath: string | null;
  clipboard: ExplorerClipboard | null;
  inline: InlineEdit | null;
  menu: ExplorerMenu | null;
  dropTarget: string | null;
  busy: Set<string>;

  select: (
    path: string,
    opts?: { additive?: boolean; range?: boolean; visible?: string[] },
  ) => void;
  clearSelection: () => void;
  setFocus: (path: string | null) => void;
  setClipboard: (clip: ExplorerClipboard | null) => void;
  setInline: (inline: InlineEdit | null) => void;
  setMenu: (menu: ExplorerMenu | null) => void;
  setDropTarget: (path: string | null) => void;
  markBusy: (paths: string[]) => void;
  unmarkBusy: (paths: string[]) => void;
  remapPaths: (from: string, to: string) => void;
}

function cloneSet(s: Set<string>): Set<string> {
  return new Set(s);
}

function remapSet(s: Set<string>, from: string, to: string): Set<string> {
  return new Set(Array.from(s).map((p) => remapPathPrefix(p, from, to)));
}

export const useExplorerStore = create<ExplorerStore>((set, get) => ({
  selected: new Set<string>(),
  anchorPath: null,
  focusPath: null,
  clipboard: null,
  inline: null,
  menu: null,
  dropTarget: null,
  busy: new Set<string>(),

  select: (path, opts) => {
    if (opts?.range && get().anchorPath && opts.visible) {
      const vis = opts.visible;
      const a = vis.indexOf(get().anchorPath!);
      const b = vis.indexOf(path);
      if (a >= 0 && b >= 0) {
        const lo = Math.min(a, b);
        const hi = Math.max(a, b);
        set({
          selected: new Set(vis.slice(lo, hi + 1)),
          focusPath: path,
        });
        return;
      }
    }
    if (opts?.additive) {
      const next = cloneSet(get().selected);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      set({ selected: next, anchorPath: path, focusPath: path });
      return;
    }
    set({
      selected: new Set([path]),
      anchorPath: path,
      focusPath: path,
    });
  },

  clearSelection: () =>
    set({ selected: new Set<string>(), anchorPath: null, focusPath: null }),

  setFocus: (path) => set({ focusPath: path }),

  setClipboard: (clipboard) => set({ clipboard }),

  setInline: (inline) => set({ inline, menu: null }),

  setMenu: (menu) => set({ menu }),

  setDropTarget: (dropTarget) => set({ dropTarget }),

  markBusy: (paths) =>
    set((s) => {
      const busy = cloneSet(s.busy);
      for (const p of paths) busy.add(p);
      return { busy };
    }),

  unmarkBusy: (paths) =>
    set((s) => {
      const busy = cloneSet(s.busy);
      for (const p of paths) busy.delete(p);
      return { busy };
    }),

  remapPaths: (from, to) =>
    set((s) => ({
      selected: remapSet(s.selected, from, to),
      busy: remapSet(s.busy, from, to),
      anchorPath: s.anchorPath ? remapPathPrefix(s.anchorPath, from, to) : null,
      focusPath: s.focusPath ? remapPathPrefix(s.focusPath, from, to) : null,
      clipboard: s.clipboard
        ? {
            ...s.clipboard,
            paths: s.clipboard.paths.map((p) => remapPathPrefix(p, from, to)),
          }
        : null,
    })),
}));
