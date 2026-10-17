/** Indentation geometry shared by tree-style rows (file tree, knowledge list). */
export const TREE_INDENT_PX = 12;
export const TREE_ROW_INSET_PX = 8;

/** Half of the 16px caret slot; guides sit on its centre. */
const CARET_CENTRE_PX = 8;

/** Left padding for a row at `depth`, leaving room for the caret slot. */
export function treeRowPaddingLeft(depth: number): number {
  return TREE_ROW_INSET_PX + depth * TREE_INDENT_PX;
}

/**
 * One 1px rule per ancestor level, lining up with the caret slots above.
 * The row must be `relative`; the rules are absolute and non-interactive.
 */
export function TreeIndentGuides({ depth }: { depth: number }) {
  if (depth <= 0) return null;
  return (
    <>
      {Array.from({ length: depth }, (_, level) => (
        <span
          key={level}
          aria-hidden
          className="pointer-events-none absolute inset-y-0 w-px bg-(--_dk-line)"
          style={{
            left: `${treeRowPaddingLeft(level) + CARET_CENTRE_PX}px`,
          }}
        />
      ))}
    </>
  );
}
