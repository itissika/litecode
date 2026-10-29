/** Box the floating panel must stay inside, in viewport coordinates. */
export type SideBox = { top: number; right: number; bottom: number; left: number };

/** Keep a portaled panel at least this far from the edges of its box. */
export const VIEWPORT_MARGIN = 8;

/**
 * Dockview lifts panel content out of `.dv-groupview` into
 * `.dv-render-overlay` and positions that overlay on the group's content box,
 * so the trigger has no groupview ancestor. Walking up to the group misses the
 * pane and the menu only meets the screen. The overlay's rect is the absolute
 * top and bottom of the pane. A groupview ancestor is the fallback for content
 * that was not relocated.
 */
export function dockPane(el: HTMLElement): HTMLElement | null {
  const overlay = el.closest<HTMLElement>(".dv-render-overlay");
  if (overlay) {
    const rect = overlay.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) return overlay;
  }
  const group = el.closest<HTMLElement>(".dv-groupview");
  if (!group) return null;
  for (const child of group.children) {
    if (child.classList.contains("dv-content-container")) {
      return child as HTMLElement;
    }
  }
  return group;
}

/**
 * Viewport clipped to the pane. Height uses this box; a pane that hangs off
 * the screen does not lend the panel that off-screen space.
 */
export function visibleBox(el: HTMLElement, vw: number, vh: number): SideBox {
  const pane = dockPane(el)?.getBoundingClientRect();
  if (!pane) return { top: 0, right: vw, bottom: vh, left: 0 };
  return {
    top: Math.max(0, pane.top),
    left: Math.max(0, pane.left),
    right: Math.min(vw, pane.right),
    bottom: Math.min(vh, pane.bottom),
  };
}

/**
 * Prefer `direction`. A finite wish keeps that side when the side can hold
 * it; otherwise the panel opens toward whichever side of the box is taller
 * (down included).
 */
export function opensUpward(
  direction: "up" | "down",
  above: number,
  below: number,
  wish: number | undefined,
): boolean {
  const holds = (room: number) =>
    wish != null && room >= wish + VIEWPORT_MARGIN;
  if (direction === "up") {
    if (holds(above)) return true;
    if (holds(below)) return false;
    return above >= below;
  }
  if (holds(below)) return false;
  if (holds(above)) return true;
  return above > below;
}
