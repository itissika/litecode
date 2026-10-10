import { subscribeWindows } from "../dockview/workbench/windows";

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
 * Listeners follow the window registry, including popouts.
 */
export function installBrowserChromeObscure(): () => void {
  let dragRelease: (() => void) | null = null;
  let menuRelease: (() => void) | null = null;
  let menuTimer: ReturnType<typeof setTimeout> | undefined;
  let dragView: Window | null = null;
  let menuView: Window | null = null;
  let menuOwner: Window | null = null;
  let startX = 0;
  let startY = 0;

  const endDrag = () => {
    dragRelease?.();
    dragRelease = null;
    const view = dragView;
    dragView = null;
    if (!view) return;
    view.removeEventListener("pointermove", onMove, true);
    view.removeEventListener("pointerup", endDrag, true);
    view.removeEventListener("pointercancel", endDrag, true);
  };

  const onMove = (event: PointerEvent) => {
    if (dragRelease) return;
    if (Math.abs(event.clientX - startX) + Math.abs(event.clientY - startY) < 4) return;
    dragRelease = pushBrowserObscure();
  };

  const clearMenu = () => {
    if (menuTimer !== undefined) {
      clearTimeout(menuTimer);
      menuTimer = undefined;
    }
    menuRelease?.();
    menuRelease = null;
    menuOwner = null;
    const view = menuView;
    menuView = null;
    if (!view) return;
    view.removeEventListener("pointerup", clearMenu, true);
    view.removeEventListener("keydown", onMenuKey, true);
  };

  const onMenuKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") clearMenu();
  };

  return subscribeWindows((entry) => {
    const view = entry.window;

    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0) return;
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (!target.closest(TAB_DRAG_TARGET)) return;
      startX = event.clientX;
      startY = event.clientY;
      endDrag();
      dragView = view;
      view.addEventListener("pointermove", onMove, true);
      view.addEventListener("pointerup", endDrag, true);
      view.addEventListener("pointercancel", endDrag, true);
    };

    const onContextMenu = () => {
      clearMenu();
      menuOwner = view;
      menuRelease = pushBrowserObscure();
      menuTimer = setTimeout(() => {
        menuTimer = undefined;
        menuView = view;
        view.addEventListener("pointerup", clearMenu, true);
        view.addEventListener("keydown", onMenuKey, true);
      }, 0);
    };

    view.addEventListener("pointerdown", onPointerDown, true);
    view.addEventListener("contextmenu", onContextMenu, true);
    return () => {
      view.removeEventListener("pointerdown", onPointerDown, true);
      view.removeEventListener("contextmenu", onContextMenu, true);
      if (dragView === view) endDrag();
      if (menuOwner === view || menuView === view) clearMenu();
    };
  });
}
