/**
 * Native browser views paint above the workbench. Anything that must appear
 * over a page — dialogs, menus, tab drags — takes a lease and the views hide
 * until every lease is released.
 */

let depth = 0;

function publish(): void {
  window.litecode?.browserSetObscured?.(depth > 0);
}

/** Returns a release function. Calling it twice is safe. */
export function pushBrowserObscure(): () => void {
  depth += 1;
  publish();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    depth = Math.max(0, depth - 1);
    publish();
  };
}

const TAB_DRAG_TARGET = ".dv-tab, .dv-void-container";

/**
 * Hide guest pages while a context menu is open, and while a tab is actually
 * being dragged. A plain click on a tab does not take a lease.
 */
export function installBrowserChromeObscure(): () => void {
  let dragRelease: (() => void) | null = null;
  let menuRelease: (() => void) | null = null;
  let menuTimer: ReturnType<typeof setTimeout> | undefined;
  let startX = 0;
  let startY = 0;

  const endDrag = () => {
    dragRelease?.();
    dragRelease = null;
    window.removeEventListener("pointermove", onMove, true);
    window.removeEventListener("pointerup", endDrag, true);
    window.removeEventListener("pointercancel", endDrag, true);
  };

  const onMove = (event: PointerEvent) => {
    if (dragRelease) return;
    if (Math.abs(event.clientX - startX) + Math.abs(event.clientY - startY) < 4) return;
    dragRelease = pushBrowserObscure();
  };

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    const target = event.target;
    if (!(target instanceof Element)) return;
    if (!target.closest(TAB_DRAG_TARGET)) return;
    startX = event.clientX;
    startY = event.clientY;
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", endDrag, true);
    window.addEventListener("pointercancel", endDrag, true);
  };

  const onMenuKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") clearMenu();
  };

  const clearMenu = () => {
    if (menuTimer !== undefined) {
      clearTimeout(menuTimer);
      menuTimer = undefined;
    }
    menuRelease?.();
    menuRelease = null;
    window.removeEventListener("pointerup", clearMenu, true);
    window.removeEventListener("keydown", onMenuKey, true);
  };

  const onContextMenu = () => {
    clearMenu();
    menuRelease = pushBrowserObscure();
    menuTimer = setTimeout(() => {
      menuTimer = undefined;
      window.addEventListener("pointerup", clearMenu, true);
      window.addEventListener("keydown", onMenuKey, true);
    }, 0);
  };

  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("contextmenu", onContextMenu, true);
  return () => {
    endDrag();
    clearMenu();
    window.removeEventListener("pointerdown", onPointerDown, true);
    window.removeEventListener("contextmenu", onContextMenu, true);
  };
}
