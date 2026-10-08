import { dockIdFromPopoutUrl, popoutPageUrl } from "./popoutUrl";

/**
 * A popped-out group is stored with the screen rectangle dockview measured
 * (`left`/`top` are `screenX`/`screenY`, `width`/`height` are the inner size).
 * On the next launch the same windows open again at that rectangle.
 * Dockview then adds the opener's `screenX`/`screenY` to `position.left`/`top`,
 * so the value handed to `fromJSON` is the saved screen point minus the
 * opener's origin.
 */

const DEFAULT_WIDTH = 960;
const DEFAULT_HEIGHT = 640;
const MIN_SPAN = 200;
const CASCADE = 36;

export type PopoutRect = { x: number; y: number; width: number; height: number };

const staged = new Map<string, PopoutRect>();

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function span(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < MIN_SPAN) return fallback;
  return Math.round(value);
}

function coord(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.round(value);
}

/**
 * Rewrite `popoutGroups` so `fromJSON` reopens each window. The returned
 * `bounds` are screen coordinates, keyed by the dock id in the popout URL.
 * A layout with nothing to reopen is returned as the same object.
 */
export function preparePopoutRestore<T>(
  layout: T,
  origin: { x: number; y: number },
): { layout: T; bounds: Map<string, PopoutRect> } {
  const bounds = new Map<string, PopoutRect>();
  if (!isRecord(layout) || !Array.isArray(layout.popoutGroups) || layout.popoutGroups.length === 0) {
    return { layout, bounds };
  }

  const next = structuredClone(layout) as T & { popoutGroups: unknown[] };
  const groups: unknown[] = [];
  for (let index = 0; index < next.popoutGroups.length; index += 1) {
    const entry = next.popoutGroups[index];
    if (!isRecord(entry) || (!isRecord(entry.data) && !isRecord(entry.grid))) continue;
    const dock =
      dockIdFromPopoutUrl(typeof entry.url === "string" ? entry.url : null) ??
      crypto.randomUUID();
    const position = isRecord(entry.position) ? entry.position : {};
    const width = span(position.width, DEFAULT_WIDTH);
    const height = span(position.height, DEFAULT_HEIGHT);
    const x = coord(position.left) ?? coord(position.x) ?? origin.x + 48 + index * CASCADE;
    const y = coord(position.top) ?? coord(position.y) ?? origin.y + 48 + index * CASCADE;
    const rect = { x, y, width, height };
    bounds.set(dock, rect);
    groups.push({
      ...entry,
      url: popoutPageUrl(dock),
      position: {
        left: rect.x - origin.x,
        top: rect.y - origin.y,
        width,
        height,
      },
    });
  }
  next.popoutGroups = groups;
  return { layout: next, bounds };
}

export function stagePopoutBounds(bounds: ReadonlyMap<string, PopoutRect>): void {
  staged.clear();
  for (const [id, rect] of bounds) staged.set(id, { ...rect });
}

/**
 * Move a just-opened popout onto the rectangle staged at restore.
 * Returns false for a window that was not part of this restore, so a popout
 * the user opens while restoration is still settling is left where they put it.
 */
export function applyStagedPopoutBounds(win: Window, dockId: string | null): boolean {
  if (!dockId || win.closed) return false;
  const rect = staged.get(dockId);
  if (!rect) return false;
  staged.delete(dockId);
  try {
    win.resizeTo(rect.width, rect.height);
    win.moveTo(rect.x, rect.y);
  } catch {
    // The window is already open. A failed snap keeps the size from window.open.
  }
  return true;
}
