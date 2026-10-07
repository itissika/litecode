/**
 * Persisted layouts must not reopen popout windows. Groups that were popped
 * out are folded back into the main grid. A snapshot with no popout groups is
 * returned as the same object so an ordinary layout is left untouched.
 */

type GridNode = {
  type?: unknown;
  data?: unknown;
  size?: unknown;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function popoutNodes(groups: unknown[]): GridNode[] {
  const nodes: GridNode[] = [];
  for (const entry of groups) {
    if (!isRecord(entry)) continue;
    if (isRecord(entry.data) && !isRecord(entry.grid)) {
      nodes.push({ type: "leaf", data: entry.data, size: 1 });
      continue;
    }
    if (isRecord(entry.grid) && isRecord(entry.grid.root)) {
      nodes.push(entry.grid.root as GridNode);
    }
  }
  return nodes;
}

export function foldPopoutsIntoGrid<T>(layout: T): T {
  if (!isRecord(layout) || !Object.prototype.hasOwnProperty.call(layout, "popoutGroups")) {
    return layout;
  }
  const groups = layout.popoutGroups;
  if (groups == null || (Array.isArray(groups) && groups.length === 0)) {
    return layout;
  }

  const next = structuredClone(layout) as T & {
    popoutGroups?: unknown;
    grid?: { root?: GridNode };
  };
  delete next.popoutGroups;
  if (!Array.isArray(groups)) return next;

  const nodes = popoutNodes(groups);
  if (nodes.length === 0) return next;
  const grid = next.grid;
  if (!isRecord(grid) || !isRecord(grid.root)) return next;
  const root = grid.root;
  if (root.type === "leaf") {
    grid.root = {
      type: "branch",
      data: [root, ...nodes],
      size: typeof root.size === "number" ? root.size : 1,
    };
    return next;
  }
  if (root.type === "branch" && Array.isArray(root.data)) {
    root.data.push(...nodes);
    return next;
  }
  return next;
}
