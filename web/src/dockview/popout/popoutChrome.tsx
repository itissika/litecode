import type { DockviewApi } from "dockview-react";

import { THEME_CHANGE_EVENT } from "../../lib/theme";
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
 * One subscription for every popout window dockview opens.
 * Theme follows `getPopouts()`. Empty render overlays are a gap in the
 * always-renderer, so those are removed here.
 */
export function bindPopoutWindows(api: DockviewApi): void {
  const watches = new Map<Window, () => void>();

  const popoutDocuments = (): Document[] => {
    const docs = [document];
    for (const popout of api.getPopouts()) {
      if (!popout.window.closed) docs.push(popout.window.document);
    }
    return docs;
  };

  const syncTheme = () => {
    const theme = currentDvTheme();
    for (const popout of api.getPopouts()) {
      if (!popout.window.closed) preparePopoutDocument(popout.window.document, theme);
    }
  };

  const unwatch = (popoutWindow: Window) => {
    const stop = watches.get(popoutWindow);
    if (!stop) return;
    watches.delete(popoutWindow);
    stop();
  };

  const watch = (popoutWindow: Window) => {
    const stopLayout = watchPopoutLayout(popoutWindow);
    const stopPin = bindPopoutPin(popoutWindow, (dockId, onTop) => {
      const set = window.litecode?.popoutSetAlwaysOnTop;
      return set ? set(dockId, onTop) : Promise.resolve(false);
    });
    return () => {
      stopLayout();
      stopPin();
    };
  };

  window.addEventListener(THEME_CHANGE_EVENT, syncTheme);
  api.onDidAddPopoutGroup((popout) => {
    if (!popout.window.closed && !watches.has(popout.window)) {
      preparePopoutDocument(popout.window.document, currentDvTheme());
      watches.set(popout.window, watch(popout.window));
    }
    releaseOrphanRenderOverlays(popoutDocuments());
  });
  api.onDidRemovePopoutGroup((popout) => unwatch(popout.window));
  api.onDidMovePanel(() => releaseOrphanRenderOverlays(popoutDocuments()));
}
