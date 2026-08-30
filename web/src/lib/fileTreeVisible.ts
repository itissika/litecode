import type { TreeEntry } from "../api/workspace";

export type FileTreeGhostKind = "newFile" | "newFolder";

export type VisibleTreeRow =
  | { type: "entry"; entry: TreeEntry; depth: number }
  | {
      type: "ghost";
      parent: string;
      depth: number;
      kind: FileTreeGhostKind;
    };

export type FileTreeGhost = {
  parent: string;
  kind: FileTreeGhostKind;
};

/**
 * Flatten the expanded explorer tree into paint order.
 * Ghost (inline create) rows sit before that parent's children, matching FileTree.
 */
export function flattenVisibleRows(
  children: Record<string, TreeEntry[] | undefined>,
  expanded: Set<string>,
  ghost: FileTreeGhost | null = null,
  parent = "",
  depth = 0,
): VisibleTreeRow[] {
  const list = children[parent] ?? [];
  const out: VisibleTreeRow[] = [];
  if (ghost && ghost.parent === parent) {
    out.push({ type: "ghost", parent, depth, kind: ghost.kind });
  }
  for (const entry of list) {
    out.push({ type: "entry", entry, depth });
    if (entry.kind === "dir" && expanded.has(entry.path)) {
      out.push(
        ...flattenVisibleRows(children, expanded, ghost, entry.path, depth + 1),
      );
    }
  }
  return out;
}

/** Paths of real entries in paint order (keyboard / range-select). */
export function visibleEntryPaths(rows: VisibleTreeRow[]): string[] {
  return rows.filter((r) => r.type === "entry").map((r) => r.entry.path);
}
