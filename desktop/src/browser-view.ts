import {
  session,
  WebContentsView,
  type BrowserWindow,
  type WebContents,
} from "electron";

import {
  BROWSER_PARTITION,
  BROWSER_STATE_CHANNEL,
  BrowserHost,
  type BrowserPage,
  type BrowserViewHandle,
} from "./browser-host";
import { isAllowedLoadUrl } from "./url-policy";

let guestSessionReady = false;

/** One persistent browser profile, separate from the workbench session. */
function prepareGuestSession(): void {
  if (guestSessionReady) return;
  const ses = session.fromPartition(BROWSER_PARTITION);
  ses.setPermissionCheckHandler(() => false);
  ses.setPermissionRequestHandler((_wc, _permission, callback) => {
    callback(false);
  });
  ses.on("will-download", (event) => {
    event.preventDefault();
  });
  guestSessionReady = true;
}

function guardGuestNavigation(wc: WebContents): void {
  const guard = (event: Electron.Event, url: string) => {
    if (!isAllowedLoadUrl(url)) event.preventDefault();
  };
  wc.on("will-navigate", guard);
  wc.on("will-redirect", guard);
}

function wrapPage(wc: WebContents): BrowserPage {
  return {
    loadURL: (url) => wc.loadURL(url),
    getURL: () => (wc.isDestroyed() ? "" : wc.getURL()),
    getTitle: () => (wc.isDestroyed() ? "" : wc.getTitle()),
    goBack: () => {
      if (!wc.isDestroyed() && wc.navigationHistory.canGoBack()) {
        wc.navigationHistory.goBack();
      }
    },
    goForward: () => {
      if (!wc.isDestroyed() && wc.navigationHistory.canGoForward()) {
        wc.navigationHistory.goForward();
      }
    },
    reload: () => {
      if (!wc.isDestroyed()) wc.reload();
    },
    stop: () => {
      if (!wc.isDestroyed()) wc.stop();
    },
    canGoBack: () => !wc.isDestroyed() && wc.navigationHistory.canGoBack(),
    canGoForward: () => !wc.isDestroyed() && wc.navigationHistory.canGoForward(),
    close: () => {
      if (!wc.isDestroyed()) wc.close();
    },
    isDestroyed: () => wc.isDestroyed(),
    onNavigate: (listener) => {
      wc.on("did-navigate", () => listener());
      wc.on("did-navigate-in-page", () => listener());
      wc.on("page-title-updated", () => listener());
    },
    onLoading: (listener) => {
      wc.on("did-start-loading", () => listener(true));
      wc.on("did-stop-loading", () => listener(false));
      wc.on("did-fail-load", () => listener(false));
    },
    setWindowOpenHandler: (handler) => {
      wc.setWindowOpenHandler(({ url }) => {
        handler(url);
        return { action: "deny" };
      });
    },
  };
}

function mountGuest(win: BrowserWindow): BrowserViewHandle {
  const view = new WebContentsView({
    webPreferences: {
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      partition: BROWSER_PARTITION,
    },
  });
  guardGuestNavigation(view.webContents);
  const page = wrapPage(view.webContents);
  let hostWindow = win;
  let attached = false;
  const detachFrom = (target: BrowserWindow) => {
    if (!attached) return;
    attached = false;
    if (!target.isDestroyed()) target.contentView.removeChildView(view);
  };
  return {
    webContents: page,
    setBounds: (bounds) => {
      if (hostWindow.isDestroyed()) return;
      view.setBounds(bounds);
    },
    setVisible: (visible) => {
      if (hostWindow.isDestroyed()) return;
      view.setVisible(visible);
    },
    setBackgroundColor: (color) => {
      view.setBackgroundColor(color);
    },
    attach: () => {
      if (attached || hostWindow.isDestroyed()) return;
      attached = true;
      hostWindow.contentView.addChildView(view);
    },
    detach: () => {
      detachFrom(hostWindow);
    },
    reparent: (target) => {
      if (!isBrowserWindow(target) || target.isDestroyed()) return false;
      if (hostWindow === target && attached) return true;
      const previous = hostWindow;
      detachFrom(previous);
      if (target.isDestroyed()) {
        if (!previous.isDestroyed()) {
          previous.contentView.addChildView(view);
          attached = true;
          hostWindow = previous;
        }
        return false;
      }
      target.contentView.addChildView(view);
      hostWindow = target;
      attached = true;
      return true;
    },
  };
}

function isBrowserWindow(value: unknown): value is BrowserWindow {
  if (!value || typeof value !== "object") return false;
  const candidate = value as BrowserWindow;
  return (
    typeof candidate.isDestroyed === "function" &&
    !!candidate.contentView &&
    typeof candidate.contentView.addChildView === "function" &&
    typeof candidate.contentView.removeChildView === "function"
  );
}

export function createBrowserHost(
  win: BrowserWindow,
  popoutWindow: (id: string) => BrowserWindow | null,
): BrowserHost {
  prepareGuestSession();
  return new BrowserHost({
    isAllowedUrl: isAllowedLoadUrl,
    onState: (state) => {
      if (win.isDestroyed()) return;
      win.webContents.send(BROWSER_STATE_CHANNEL, state);
    },
    createView: () => mountGuest(win),
    moveView: (view, popoutId) => {
      if (popoutId === null) return view.reparent(win);
      const target = popoutWindow(popoutId);
      if (!target || target.isDestroyed()) return false;
      return view.reparent(target);
    },
  });
}
