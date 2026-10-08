import { isCenterDock } from "../popout/location";

/** Center tabs live on the main grid or in a popout. Edge tabs are the side rails. */
export type TabClass = "center" | "edge";

/** Where a dock drop is aimed. A missing group is the center grid's own outer split. */
export type DockTarget = "center" | "edge" | "root";

export type DockDecision = "allow" | "reject";

/** What a center-tab release does after Dockview has had its chance to accept the drop. */
export type ReleaseDecision = "stay" | "join" | "popout";

export interface WindowBox {
  screenX: number;
  screenY: number;
  outerWidth: number;
  outerHeight: number;
}

export interface ScreenWindow {
  key: string;
  box: WindowBox;
}

/** Screen key of the main window. Popout keys are their own. */
export const MAIN_WINDOW_KEY = "main-window";

export function tabClass(locationType: string | undefined): TabClass | null {
  if (locationType === "edge") return "edge";
  if (isCenterDock(locationType)) return "center";
  return null;
}

export function dockTarget(locationType: string | undefined): DockTarget {
  if (locationType === "edge") return "edge";
  if (isCenterDock(locationType)) return "center";
  return "root";
}

/**
 * Center tabs land on the center grid and on popout windows.
 * Edge tabs land only on edge rails.
 * A root target is the center grid's outer split, so center tabs may use it.
 */
export function decideDockTarget(source: TabClass, target: DockTarget): DockDecision {
  if (source === "center") return target === "edge" ? "reject" : "allow";
  return target === "edge" ? "allow" : "reject";
}

/** Unknown drags are left to Dockview. A known class is rejected when the target is the other zone. */
export function rejectsDockTarget(
  sourceType: string | undefined,
  targetType: string | undefined,
): boolean {
  const source = tabClass(sourceType);
  if (!source) return false;
  return decideDockTarget(source, dockTarget(targetType)) === "reject";
}

/**
 * Center tabs open a new window only when the pointer is outside every
 * LiteCode window. A point inside an existing popout joins that window.
 * Edge tabs never open or join a window.
 * `windowKey` is null when the point hits no window.
 */
export function decideRelease(
  source: TabClass | null,
  windowKey: string | null,
  mainKey: string = MAIN_WINDOW_KEY,
): ReleaseDecision {
  if (source !== "center") return "stay";
  if (windowKey === null) return "popout";
  if (windowKey === mainKey) return "stay";
  return "join";
}

export function containsPoint(
  box: WindowBox,
  point: { screenX: number; screenY: number },
): boolean {
  if (!Number.isFinite(point.screenX) || !Number.isFinite(point.screenY)) return false;
  if (!Number.isFinite(box.screenX) || !Number.isFinite(box.screenY)) return false;
  if (!Number.isFinite(box.outerWidth) || !Number.isFinite(box.outerHeight)) return false;
  if (box.outerWidth <= 0 || box.outerHeight <= 0) return false;
  return (
    point.screenX >= box.screenX &&
    point.screenY >= box.screenY &&
    point.screenX < box.screenX + box.outerWidth &&
    point.screenY < box.screenY + box.outerHeight
  );
}

/**
 * The window under the pointer. Overlapping windows take the smaller one,
 * which is the popout sitting on top of a larger window.
 */
export function windowAtPoint(
  point: { screenX: number; screenY: number },
  windows: ScreenWindow[],
): string | null {
  let best: { key: string; area: number } | null = null;
  for (const win of windows) {
    if (!containsPoint(win.box, point)) continue;
    const area = win.box.outerWidth * win.box.outerHeight;
    if (!best || area < best.area) best = { key: win.key, area };
  }
  return best?.key ?? null;
}
