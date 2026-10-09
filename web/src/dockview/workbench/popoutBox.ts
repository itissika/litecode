/**
 * Screen rectangle for a new popout.
 *
 * Dockview's own box uses the opener's `window.screenX` / `screenY`. On
 * Windows a minimized main window reports those as about -32000, so a tab
 * dragged out of a visible popout opens off-screen: the taskbar preview
 * shows it, and restoring the main window does not bring it back.
 * A release point is already in screen coordinates. Otherwise the box is
 * the group rectangle inside the window that actually holds the tab.
 */

const DEFAULT_WIDTH = 960;
const DEFAULT_HEIGHT = 640;

export interface PopoutScreenBox {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface PopoutHostBox {
  screenX: number;
  screenY: number;
  innerWidth: number;
  innerHeight: number;
}

export interface PopoutGroupRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

function span(value: number | undefined, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 1) return fallback;
  return Math.round(value);
}

export function popoutScreenBox(
  host: PopoutHostBox | null,
  rect: PopoutGroupRect | null,
  at?: { screenX: number; screenY: number } | null,
): PopoutScreenBox | undefined {
  const width = span(rect?.width, span(host?.innerWidth, DEFAULT_WIDTH));
  const height = span(rect?.height, span(host?.innerHeight, DEFAULT_HEIGHT));
  if (at && Number.isFinite(at.screenX) && Number.isFinite(at.screenY)) {
    return {
      left: Math.round(at.screenX),
      top: Math.round(at.screenY),
      width,
      height,
    };
  }
  if (!host || !Number.isFinite(host.screenX) || !Number.isFinite(host.screenY)) return undefined;
  const originX = Number.isFinite(rect?.left) ? rect!.left : 0;
  const originY = Number.isFinite(rect?.top) ? rect!.top : 0;
  return {
    left: Math.round(host.screenX + originX),
    top: Math.round(host.screenY + originY),
    width,
    height,
  };
}
