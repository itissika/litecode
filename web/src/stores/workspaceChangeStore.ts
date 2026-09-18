import { create } from "zustand";

import type { WorkspaceChangeKind } from "../api/workspace";

/** Latest workspace change tick, for panels that must re-read a file. */
export interface WorkspaceChangeTick {
  seq: number;
  paths: string[];
  kind: WorkspaceChangeKind;
}

interface WorkspaceChangeStore {
  last: WorkspaceChangeTick | null;
  record: (paths: string[], kind: WorkspaceChangeKind) => void;
}

let nextSeq = 0;

/**
 * One-slot bus for `workspace/changed`: consumers that are not the editor or
 * the explorer (e.g. the plan panel) re-read their file when their path is hit.
 */
export const useWorkspaceChangeStore = create<WorkspaceChangeStore>((set) => ({
  last: null,
  record: (paths, kind) =>
    set({ last: { seq: ++nextSeq, paths: [...paths], kind } }),
}));
