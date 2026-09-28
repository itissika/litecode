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
