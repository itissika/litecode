import { dockIdFromPopoutUrl } from "../config/popoutUrl";

/**
 * Dockview's own location mark.
 * `grid` is the main window's center. `popout` is that same center living in
 * another window. `edge` is a side rail. Drag treats grid and popout as one
 * dock; new panels still open on the main grid.
 */
export function isMainGrid(type: string | undefined): boolean {
  return type === "grid";
}

export function isCenterDock(type: string | undefined): boolean {
  return type === "grid" || type === "popout";
}

/** Electron window id carried in the popout URL dockview stores on the group. */
export function dockIdFromLocation(location: {
  type: string;
  popoutUrl?: string;
} | undefined): string | null {
  if (location?.type !== "popout") return null;
  return dockIdFromPopoutUrl(location.popoutUrl);
}
