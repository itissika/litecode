import { THEME_CHANGE_EVENT } from "../../lib/theme";
import { getWindows, registeredDocuments, subscribeWindows } from "../workbench/windows";
import { releaseOrphanRenderOverlays } from "./orphanOverlay";
import { bindPopoutPin } from "./popoutPin";

const LAYOUT_WATCH_MS = 100;

/** Marks a popout document so its tab-bar void stays a window drag handle. */
export const POPOUT_DOCUMENT_CLASS = "litecode-popout";

/** Copy the main document's dockview theme onto a popout document. */
export function preparePopoutDocument(doc: Document, dvTheme: string): void {
  doc.documentElement.setAttribute("data-dv-theme", dvTheme);
  doc.documentElement.classList.add("litecode-dv-base", POPOUT_DOCUMENT_CLASS);
  doc.body.style.margin = "0";
  doc.body.style.background = "var(--_dk-root)";
}

function currentDvTheme(): string {
  return document.documentElement.getAttribute("data-dv-theme") ?? "dark";
}

/**
 * Dockview lays a popout out on the child window's `resize` event, and that
 * event is dropped intermittently. Poll the content box from the popout
 * window itself — the opener is in the background while the user resizes —
 * and re-dispatch `resize` so Dockview's own listener runs.
 */
export function watchPopoutLayout(win: Window): () => void {
  let width = win.innerWidth;
  let height = win.innerHeight;
  const timer = win.setInterval(() => {
    if (win.closed) {
      win.clearInterval(timer);
      return;
    }
    const nextWidth = win.innerWidth;
    const nextHeight = win.innerHeight;
    if (nextWidth === width && nextHeight === height) return;
    width = nextWidth;
    height = nextHeight;
    if (nextWidth > 0 && nextHeight > 0) win.dispatchEvent(new Event("resize"));
  }, LAYOUT_WATCH_MS);
  return () => win.clearInterval(timer);
}

/**
 * Theme, pin, and resize watch for every registered popout.
 * Empty render overlays are a gap in the always-renderer, so those are
 * removed when a popout registers. Panel moves are handled by the workbench.
 */
export function bindPopoutWindows(): () => void {
  const syncTheme = () => {
    const theme = currentDvTheme();
    for (const entry of getWindows()) {
      if (!entry.dockId) continue;
      try {
        if (!entry.window.closed) preparePopoutDocument(entry.window.document, theme);
      } catch {
        // The popout is already leaving the registry.
      }
    }
  };

  window.addEventListener(THEME_CHANGE_EVENT, syncTheme);
  const stop = subscribeWindows((entry) => {
    if (!entry.dockId) return () => {};
    try {
      if (entry.window.closed) return () => {};
      preparePopoutDocument(entry.window.document, currentDvTheme());
    } catch {
      return () => {};
    }
    releaseOrphanRenderOverlays(registeredDocuments());
    const stopLayout = watchPopoutLayout(entry.window);
    const stopPin = bindPopoutPin(entry.window, (dockId, onTop) => {
      const set = window.litecode?.popoutSetAlwaysOnTop;
      return set ? set(dockId, onTop) : Promise.resolve(false);
    });
    return () => {
      stopLayout();
      stopPin();
    };
  });
  return () => {
    window.removeEventListener(THEME_CHANGE_EVENT, syncTheme);
    stop();
  };
}
