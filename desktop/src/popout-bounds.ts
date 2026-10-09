export type PopoutBounds = { x: number; y: number; width: number; height: number };

function overlaps(a: PopoutBounds, b: PopoutBounds): boolean {
  return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
}

/**
 * Keep a popout on a display. A minimized opener reports a screen origin
 * around -32000, and a window opened there stays off-screen after the
 * opener is restored. Rectangles that already meet a work area are kept,
 * including a monitor to the left of the origin.
 */
export function boundsOnDisplay(
  bounds: PopoutBounds,
  areas: readonly PopoutBounds[],
  anchor: { x: number; y: number },
): PopoutBounds {
  if (areas.length === 0 || areas.some((area) => overlaps(bounds, area))) return bounds;
  const home =
    areas.find(
      (area) =>
        anchor.x >= area.x &&
        anchor.x < area.x + area.width &&
        anchor.y >= area.y &&
        anchor.y < area.y + area.height,
    ) ?? areas[0];
  if (!home) return bounds;
  const width = Math.min(bounds.width, home.width);
  const height = Math.min(bounds.height, home.height);
  let x = anchor.x;
  let y = anchor.y;
  if (x + width > home.x + home.width) x = home.x + home.width - width;
  if (y + height > home.y + home.height) y = home.y + home.height - height;
  if (x < home.x) x = home.x;
  if (y < home.y) y = home.y;
  return {
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(width),
    height: Math.round(height),
  };
}

/**
 * Screen rectangle dockview puts on `window.open`. `left`/`top` are screen
 * coordinates. Electron's override options replace that feature string, so the
 * host has to copy the rectangle onto the BrowserWindow itself.
 */
export function popoutBoundsFromFeatures(features: string | undefined): PopoutBounds | null {
  if (!features) return null;
  const values = new Map<string, number>();
  for (const part of features.split(",")) {
    const eq = part.indexOf("=");
    if (eq <= 0) continue;
    const key = part.slice(0, eq).trim().toLowerCase();
    const raw = Number(part.slice(eq + 1).trim());
    if (!Number.isFinite(raw)) continue;
    values.set(key, raw);
  }
  const x = values.get("left") ?? values.get("x");
  const y = values.get("top") ?? values.get("y");
  const width = values.get("width");
  const height = values.get("height");
  if (x === undefined || y === undefined || width === undefined || height === undefined) return null;
  if (width < 1 || height < 1) return null;
  return {
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(width),
    height: Math.round(height),
  };
}
