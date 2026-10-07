import { useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { DockviewApi } from "dockview-react";

import { THEME_CHANGE_EVENT } from "../../lib/theme";
import { TitleBar } from "../shell/TitleBar";
import { dockIdFromLocation } from "./location";
import { releaseOrphanRenderOverlays } from "./orphanOverlay";

const TITLEBAR_ID = "litecode-popout-titlebar";
const SHELL_ID = "dv-popout-window";
const TITLEBAR_PX = 32;

/** Copy the main document's dockview theme onto a popout document. */
export function preparePopoutDocument(doc: Document, dvTheme: string): void {
  doc.documentElement.setAttribute("data-dv-theme", dvTheme);
  doc.documentElement.classList.add("litecode-dv-base");
  doc.body.style.margin = "0";
  doc.body.style.background = "var(--_dk-root)";
  const shell = doc.getElementById(SHELL_ID);
  if (shell instanceof HTMLElement) {
    shell.style.top = `${TITLEBAR_PX}px`;
    shell.style.height = `calc(100% - ${TITLEBAR_PX}px)`;
  }
}

function currentDvTheme(): string {
  return document.documentElement.getAttribute("data-dv-theme") ?? "dark";
}

function readSessionMode(): "local" | "remote" {
  return window.litecode?.getSessionMode?.() === "remote" ? "remote" : "local";
}

function PopoutTitleBar({ dockId }: { dockId: string }) {
  const [maximized, setMaximized] = useState(false);
  const sessionMode = readSessionMode();

  useEffect(() => {
    void window.litecode?.popoutWindowIsMaximized?.(dockId).then(setMaximized);
  }, [dockId]);

  return (
    <TitleBar
      sessionMode={sessionMode}
      showMenu={false}
      chrome={{
        maximized,
        onMinimize: () => {
          void window.litecode?.popoutWindowMinimize?.(dockId);
        },
        onToggleMaximize: () => {
          void window.litecode
            ?.popoutWindowMaximizeToggle?.(dockId)
            .then(setMaximized);
        },
        onClose: () => {
          void window.litecode?.popoutWindowClose?.(dockId);
        },
      }}
    />
  );
}

function mountTitleBar(popoutWindow: Window, dockId: string): Root | null {
  if (popoutWindow.closed) return null;
  const doc = popoutWindow.document;
  if (!doc.body || !doc.getElementById(SHELL_ID)) return null;
  preparePopoutDocument(doc, currentDvTheme());
  if (doc.getElementById(TITLEBAR_ID)) return null;
  const host = doc.createElement("div");
  host.id = TITLEBAR_ID;
  doc.body.prepend(host);
  const root = createRoot(host);
  root.render(<PopoutTitleBar dockId={dockId} />);
  return root;
}

/**
 * One subscription for every popout window dockview opens.
 * The title bar and theme follow `getPopouts()`. The dock id is the one
 * dockview records on the group (`location.popoutUrl`). Empty render
 * overlays are a gap in the always-renderer, so those are removed here.
 */
export function bindPopoutWindows(api: DockviewApi): void {
  const roots = new Map<Window, Root>();

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

  const unmount = (popoutWindow: Window) => {
    const root = roots.get(popoutWindow);
    if (!root) return;
    roots.delete(popoutWindow);
    root.unmount();
  };

  window.addEventListener(THEME_CHANGE_EVENT, syncTheme);
  api.onDidAddPopoutGroup((popout) => {
    const dockId = dockIdFromLocation(popout.group.api.location);
    if (!dockId || roots.has(popout.window)) {
      releaseOrphanRenderOverlays(popoutDocuments());
      return;
    }
    const root = mountTitleBar(popout.window, dockId);
    if (root) roots.set(popout.window, root);
    releaseOrphanRenderOverlays(popoutDocuments());
  });
  api.onDidRemovePopoutGroup((popout) => unmount(popout.window));
  api.onDidMovePanel(() => releaseOrphanRenderOverlays(popoutDocuments()));
}
