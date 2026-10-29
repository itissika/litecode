import {
  session,
  WebContentsView,
  type BrowserWindow,
  type WebContents,
} from "electron";

import { APP_VIEW_STATE_CHANNEL, BROWSER_STATE_CHANNEL } from "./bridge-contract";
import {
  BROWSER_PARTITION,
  BrowserHost,
  type BrowserPage,
  type BrowserViewHandle,
} from "./browser-host";
import { t } from "./i18n";
import { isAllowedLoadUrl } from "./url-policy";

let guestSessionReady = false;
const downloadPolicyPartitions = new Set<string>();

function mediaAllowed(
  permissions: readonly string[],
  mediaTypes: readonly ("audio" | "video")[] | undefined,
): boolean {
  if (!mediaTypes || mediaTypes.length === 0) {
    // Chromium sometimes performs a generic media check before the concrete
    // request. Keep the configured device family available.
    return permissions.includes("microphone") || permissions.includes("camera");
  }
  return mediaTypes.every((type) =>
    type === "audio"
      ? permissions.includes("microphone")
      : permissions.includes("camera"),
  );
}

function permissionAllowed(
  permission: string,
  permissions: readonly string[],
  mediaTypes?: readonly ("audio" | "video")[],
): boolean {
  if (permission === "media") {
    return mediaAllowed(permissions, mediaTypes);
  }
  if (permission === "clipboard-sanitized-write") {
    return permissions.includes("clipboard-write");
  }
  return permissions.includes(permission);
}

function prepareSession(
  partition: string,
  permissions: readonly string[],
  allowDownloads: boolean,
): void {
  const ses = session.fromPartition(partition);
  ses.setPermissionCheckHandler((_wc, permission, _origin, details) => {
    const mediaTypes =
      permission === "media" &&
      "mediaType" in details &&
      (details.mediaType === "audio" || details.mediaType === "video")
        ? [details.mediaType]
        : undefined;
    return permissionAllowed(permission, permissions, mediaTypes);
  });
  ses.setPermissionRequestHandler((_wc, permission, callback, details) => {
    const mediaTypes =
      permission === "media" &&
      "mediaTypes" in details &&
      Array.isArray(details.mediaTypes)
        ? details.mediaTypes
        : undefined;
    callback(permissionAllowed(permission, permissions, mediaTypes));
  });
  // A partition is long-lived and app views may be recreated; register the
  // download policy once, not once per view.
  if (!downloadPolicyPartitions.has(partition)) {
    downloadPolicyPartitions.add(partition);
    ses.on("will-download", (event) => {
      if (!allowDownloads) event.preventDefault();
    });
  }
}

/** One persistent browser profile, separate from the workbench session. */
function prepareGuestSession(): void {
  if (guestSessionReady) return;
  prepareSession(BROWSER_PARTITION, [], false);
  guestSessionReady = true;
}

function guardGuestNavigation(
  wc: WebContents,
  allowUrl: (url: string) => boolean,
  onExternal?: (url: string) => void,
): void {
  const guard = (event: Electron.Event, url: string) => {
    if (allowUrl(url)) return;
    event.preventDefault();
    if (onExternal && (url.startsWith("http://") || url.startsWith("https://"))) {
      onExternal(url);
    }
  };
  wc.on("will-navigate", guard);
  wc.on("will-redirect", guard);
}

function wrapPage(wc: WebContents): BrowserPage {
  let inputBlocked = false;
  wc.on("before-input-event", (event) => {
    if (inputBlocked) event.preventDefault();
  });
  wc.on("before-mouse-event", (event) => {
    if (inputBlocked) event.preventDefault();
  });
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
    executeJavaScript: (code) => wc.executeJavaScript(code, true),
    setInputBlocked: (blocked) => {
      inputBlocked = blocked;
    },
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

function mountGuest(
  win: BrowserWindow,
  partition: string,
  allowUrl: (url: string) => boolean,
  onExternal?: (url: string) => void,
): BrowserViewHandle {
  const view = new WebContentsView({
    webPreferences: {
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      partition,
    },
  });
  guardGuestNavigation(view.webContents, allowUrl, onExternal);
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
    createView: () => mountGuest(win, BROWSER_PARTITION, isAllowedLoadUrl),
    fallbackTitle: t("browser.title"),
    moveView: (view, popoutId) => {
      if (popoutId === null) return view.reparent(win);
      const target = popoutWindow(popoutId);
      if (!target || target.isDestroyed()) return false;
      return view.reparent(target);
    },
  });
}

export const APP_VIEW_ID_PATTERN = /^app-[a-z0-9][a-z0-9-]{0,63}$/;

const appSessionPermissions = new Map<string, readonly string[]>();

/** App views keep their own persistent partition, permission allowlist, and
 * downloads. The partition is derived from the workspace/app view key. */
export function configureAppSession(partition: string, permissions: readonly string[]): void {
  appSessionPermissions.set(partition, permissions);
  prepareSession(partition, permissions, true);
}

export type AppViewHostOptions = {
  partitionFor: (id: string) => string;
  isAllowedUrl: (url: string, id: string) => boolean;
  onExternal: (url: string) => void;
};

/** A second BrowserHost for local app pages. It has no popout host movement. */
export function createAppViewHost(win: BrowserWindow, options: AppViewHostOptions): BrowserHost {
  return new BrowserHost({
    idPattern: APP_VIEW_ID_PATTERN,
    isAllowedUrl: options.isAllowedUrl,
    createView: (id) => {
      const partition = options.partitionFor(id);
      prepareSession(partition, appSessionPermissions.get(partition) ?? [], true);
      return mountGuest(
        win,
        partition,
        (url) => options.isAllowedUrl(url, id),
        options.onExternal,
      );
    },
    onWindowOpen: (_id, url) => options.onExternal(url),
    onState: (state) => {
      if (!win.isDestroyed()) win.webContents.send(APP_VIEW_STATE_CHANNEL, state);
    },
    fallbackTitle: "App",
  });
}
