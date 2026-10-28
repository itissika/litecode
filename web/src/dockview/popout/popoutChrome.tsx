import { THEME_CHANGE_EVENT } from "../../lib/theme";
import { getWindows, registeredDocuments, subscribeWindows } from "../workbench/windows";
import { releaseOrphanRenderOverlays } from "./orphanOverlay";
import { bindPopoutActions, type PopoutActionsHost } from "./popoutActions";
import { mirrorDocumentStyles } from "./popoutStyles";

const LAYOUT_WATCH_MS = 100;

/** Marks a popout document so its tab-bar void stays a window drag handle. */
const POPOUT_DOCUMENT_CLASS = "litecode-popout";

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
 * The popout has no preload, and host IPC only accepts the main window as the
 * sender, so its controls call back through this renderer with the dock id.
 * A plain browser has no bridge: pin and close-all still work, the OS window
 * buttons are not rendered.
 */
function popoutActionsHost(): PopoutActionsHost {
  const bridge = window.litecode;
  // An older dev preload may not carry the window channels yet.
  const canControl =
    !!bridge &&
    typeof bridge.popoutWindowMinimize === "function" &&
    typeof bridge.popoutWindowMaximizeToggle === "function" &&
    typeof bridge.popoutWindowIsMaximized === "function";
  const control = canControl
    ? {
        minimize: (dockId: string) => bridge.popoutWindowMinimize(dockId),
        toggleMaximize: (dockId: string) => bridge.popoutWindowMaximizeToggle(dockId),
        isMaximized: (dockId: string) => bridge.popoutWindowIsMaximized(dockId),
      }
    : undefined;
  return {
    setAlwaysOnTop: (dockId, onTop) =>
      bridge?.popoutSetAlwaysOnTop?.(dockId, onTop) ?? Promise.resolve(false),
    windowControl: control,
  };
}

/**
 * Theme, window controls, and resize watch for every registered popout.
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
    let stopStyles = () => {};
    try {
      if (entry.window.closed) return () => {};
      preparePopoutDocument(entry.window.document, currentDvTheme());
      // Dockview copies the opener's sheets once, at load. A panel that mounts
      // later (restored popout, lazy CSS chunk, Monaco's theme sheet) needs the
      // head mirrored for as long as the window lives.
      stopStyles = mirrorDocumentStyles(entry.window.document);
    } catch {
      return () => {};
    }
    releaseOrphanRenderOverlays(registeredDocuments());
    const stopLayout = watchPopoutLayout(entry.window);
    const stopActions = bindPopoutActions(entry.window, popoutActionsHost());
    return () => {
      stopStyles();
      stopLayout();
      stopActions();
    };
  });
  return () => {
    window.removeEventListener(THEME_CHANGE_EVENT, syncTheme);
    stop();
  };
}
