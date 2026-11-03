/** Shared multi-select mechanics for list/tree surfaces.

    The store owns the data; this module owns the four decisions every surface
    makes the same way: replace, toggle, range, and anchor preservation. It does
    not know about files, git rows, or visibility rules. */

export interface ListSelectionOptions {
  /** Ctrl/Cmd-click: toggle the row in the current selection. */
  additive?: boolean;
  /** Shift-click: select the inclusive range between anchor and row. */
  range?: boolean;
  /** Rows in display order. Range selection is a no-op without it. */
  visible?: readonly string[];
}

export interface ListSelection {
  selected: Set<string>;
  /** Anchor for the next range selection. */
  anchor: string | null;
}

/**
 * Apply one selection gesture to a list.
 *
 * - A valid range keeps the existing anchor, matching VS Code and both
 *   current stores.
 * - Additive toggles the row and moves the anchor to it.
 * - A plain click replaces the selection and moves the anchor.
 * - A range request that cannot resolve (no anchor, missing row, or no
 *   visible order) falls through to the additive/plain behavior.
 */
export function applyListSelection(
  current: ReadonlySet<string>,
  id: string,
  anchor: string | null,
  opts: ListSelectionOptions = {},
): ListSelection {
  const visible = opts.visible;
  if (opts.range && anchor != null && visible) {
    const from = visible.indexOf(anchor);
    const to = visible.indexOf(id);
    if (from >= 0 && to >= 0) {
      const lo = Math.min(from, to);
      const hi = Math.max(from, to);
      return {
        selected: new Set(visible.slice(lo, hi + 1)),
        anchor,
      };
    }
  }

  if (opts.additive) {
    const selected = new Set(current);
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    return { selected, anchor: id };
  }

  return { selected: new Set([id]), anchor: id };
}
