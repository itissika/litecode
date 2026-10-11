export type ScrollBox = {
  top: number;
  bottom: number;
  height: number;
};

/**
 * Pixels to add to one scroller so `child` sits inside `parent`.
 * Same alignment as `scrollIntoView({ block: "nearest" })`: the nearer edge,
 * or the top when the child is taller than the parent.
 * Add this to that scroller's `scrollTop` only. `scrollIntoView` also scrolls
 * every ancestor, including `overflow: hidden` shells such as dockview groups.
 */
export function nearestScrollDelta(
  parent: ScrollBox,
  child: ScrollBox,
): number {
  if (child.height >= parent.height) return child.top - parent.top;
  if (child.top < parent.top) return child.top - parent.top;
  if (child.bottom > parent.bottom) return child.bottom - parent.bottom;
  return 0;
}

/** Whether `child` is fully visible inside `scrollParent`'s viewport. */
export function isVisibleInScrollParent(
  scrollParent: HTMLElement,
  child: Element,
  padding = 2,
): boolean {
  const parent = scrollParent.getBoundingClientRect();
  const box = child.getBoundingClientRect();
  return (
    box.top >= parent.top + padding &&
    box.bottom <= parent.bottom - padding &&
    box.left >= parent.left + padding &&
    box.right <= parent.right - padding
  );
}
