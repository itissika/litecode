export type PopoutBounds = { x: number; y: number; width: number; height: number };

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
