const DOCK_ID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Same-origin page dockview opens for a grid popout. `dock` identifies the window. */
export function popoutPageUrl(dockId: string = crypto.randomUUID()): string {
  return `/popout.html?dock=${dockId}`;
}

/** Dock id embedded in a popout page URL, or null when the URL is not one of ours. */
export function dockIdFromPopoutUrl(raw: string | undefined | null): string | null {
  if (!raw) return null;
  let url: URL;
  try {
    url = new URL(raw, "http://127.0.0.1");
  } catch {
    return null;
  }
  if (url.pathname !== "/popout.html" || url.hash) return null;
  const keys = [...url.searchParams.keys()];
  if (keys.length !== 1 || keys[0] !== "dock") return null;
  const dock = url.searchParams.get("dock");
  if (!dock || !DOCK_ID.test(dock)) return null;
  return dock;
}
