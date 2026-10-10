import { dockIdFromPopoutUrl } from "../config/popoutUrl";
import { dockIdFromLocation } from "../popout/location";
import type { GroupLike } from "./readGroup";

/**
 * One workbench window. The main window has a null dock id. A popout's dock id
 * is the id in `/popout.html?dock=`, shared with the main process BrowserWindow
 * map. The two maps are not the same object: this one is the DOM window.
 *
 * A handle may exist on the other side before this record does. Callers that
 * need the OS window fail closed; they do not wait.
 */
export interface WorkbenchWindow {
  dockId: string | null;
  window: Window;
  groupId: string | null;
  group: GroupLike | null;
}

interface PopoutRef {
  id: string;
  window: Window;
  group: GroupLike;
}

interface PopoutApi {
  onDidAddPopoutGroup: (cb: (popout: PopoutRef) => void) => { dispose(): void };
  onDidRemovePopoutGroup: (cb: (popout: PopoutRef) => void) => { dispose(): void };
  getPopouts?: () => PopoutRef[];
}

const MAIN_KEY = "";

const windows = new Map<string, WorkbenchWindow>();
const pending = new Map<string, { stop: () => void }>();
const registerListeners = new Set<(entry: WorkbenchWindow) => void>();
const unregisterListeners = new Set<(entry: WorkbenchWindow) => void>();

let mainWindow: Window = window;
let activeStop: (() => void) | null = null;
let readPopouts: (() => PopoutRef[]) | null = null;

function keyOf(dockId: string | null): string {
  return dockId ?? MAIN_KEY;
}

function emitRegister(entry: WorkbenchWindow): void {
  for (const listener of registerListeners) listener(entry);
}

function emitUnregister(entry: WorkbenchWindow): void {
  for (const listener of unregisterListeners) listener(entry);
}

function admit(entry: WorkbenchWindow): void {
  const key = keyOf(entry.dockId);
  if (windows.has(key)) return;
  windows.set(key, entry);
  emitRegister(entry);
}

function dropEntry(entry: WorkbenchWindow): void {
  const key = keyOf(entry.dockId);
  if (windows.get(key) !== entry) return;
  windows.delete(key);
  emitUnregister(entry);
}

export function onDidRegisterWindow(
  listener: (entry: WorkbenchWindow) => void,
): () => void {
  registerListeners.add(listener);
  return () => {
    registerListeners.delete(listener);
  };
}

export function onDidUnregisterWindow(
  listener: (entry: WorkbenchWindow) => void,
): () => void {
  unregisterListeners.add(listener);
  return () => {
    unregisterListeners.delete(listener);
  };
}

/** Attach to every current window, and to each one that registers later. */
export function subscribeWindows(
  attach: (entry: WorkbenchWindow) => () => void,
): () => void {
  const stops = new Map<WorkbenchWindow, () => void>();
  const onAdd = (entry: WorkbenchWindow) => {
    if (stops.has(entry)) return;
    stops.set(entry, attach(entry));
  };
  const onRemove = (entry: WorkbenchWindow) => {
    const stop = stops.get(entry);
    if (!stop) return;
    stops.delete(entry);
    stop();
  };
  const offAdd = onDidRegisterWindow(onAdd);
  const offRemove = onDidUnregisterWindow(onRemove);
  for (const entry of windows.values()) onAdd(entry);
  return () => {
    offAdd();
    offRemove();
    for (const stop of stops.values()) stop();
    stops.clear();
  };
}

export function getWindows(): readonly WorkbenchWindow[] {
  const listed = readPopouts?.() ?? [];
  for (const popout of listed) {
    if (!scriptable(popout.window)) continue;
    const dockId = dockIdOf(popout);
    if (!dockId) continue;
    const entry = windows.get(dockId);
    if (!entry) continue;
    entry.group = popout.group;
    entry.groupId = popout.group.api.id ?? popout.id;
    entry.window = popout.window;
  }
  return [...windows.values()];
}

/** The window that owns `node`, or the main window when it cannot be resolved. */
export function getWindow(node: Node | Event | null | undefined): Window {
  if (node && typeof node === "object" && "ownerDocument" in node) {
    const view = (node as Node).ownerDocument?.defaultView;
    if (view) return view;
  }
  if (node && typeof node === "object" && "view" in node) {
    const view = (node as Event & { view?: Window | null }).view;
    if (view) return view;
  }
  return mainWindow;
}

export function registeredDocuments(): Document[] {
  const docs: Document[] = [];
  for (const entry of windows.values()) {
    try {
      if (!entry.window.closed) docs.push(entry.window.document);
    } catch {
      // A window that has gone unscriptable is already leaving the table.
    }
  }
  return docs;
}

function scriptable(win: Window): boolean {
  try {
    return win.closed !== true && win.document != null;
  } catch {
    return false;
  }
}

function pendingKey(popout: PopoutRef): string {
  return popout.group.api.id ?? popout.id;
}

function dockIdOf(popout: PopoutRef): string | null {
  const fromGroup = dockIdFromLocation(
    popout.group.api.location as { type: string; popoutUrl?: string } | undefined,
  );
  if (fromGroup) return fromGroup;
  try {
    return dockIdFromPopoutUrl(popout.window.location?.href);
  } catch {
    return null;
  }
}

function dropPending(popout: PopoutRef): void {
  const item = pending.get(pendingKey(popout));
  if (!item) return;
  pending.delete(pendingKey(popout));
  item.stop();
}

/**
 * Returns true when the popout is registered, already registered, or cannot
 * be registered. False means the dock id is not on the group yet.
 */
function admitPopout(popout: PopoutRef): boolean {
  if (!scriptable(popout.window)) {
    dropPending(popout);
    return true;
  }
  const dockId = dockIdOf(popout);
  if (!dockId) return false;
  dropPending(popout);
  if (windows.has(dockId)) return true;
  admit({
    dockId,
    window: popout.window,
    groupId: popout.group.api.id ?? popout.id,
    group: popout.group,
  });
  return true;
}

function watchUntilIdentified(popout: PopoutRef): void {
  const key = pendingKey(popout);
  if (pending.has(key)) return;
  let stopLoad = () => {};
  try {
    const onLoad = () => {
      admitPopout(popout);
    };
    popout.window.addEventListener("load", onLoad);
    stopLoad = () => {
      try {
        popout.window.removeEventListener("load", onLoad);
      } catch {
        // The window closed before the dock id arrived.
      }
    };
  } catch {
    // The load event is unreachable. The timer below is the other chance.
  }
  const timer = setTimeout(() => {
    admitPopout(popout);
  }, 0);
  pending.set(key, {
    stop: () => {
      clearTimeout(timer);
      stopLoad();
    },
  });
}

function removePopout(popout: PopoutRef): void {
  dropPending(popout);
  const dockId = scriptable(popout.window) ? dockIdOf(popout) : null;
  const entry =
    (dockId ? windows.get(dockId) : undefined) ??
    [...windows.values()].find((item) => item.dockId !== null && item.window === popout.window);
  if (entry?.dockId) dropEntry(entry);
}

/**
 * Register the main window and every popout Dockview opens.
 * Safe to call again: the previous binding is released first, so the main
 * window stays a single record.
 */
export function bindWindowRegistry(api: PopoutApi, main: Window = window): () => void {
  activeStop?.();
  mainWindow = main;
  readPopouts = typeof api.getPopouts === "function" ? () => api.getPopouts?.() ?? [] : null;
  admit({ dockId: null, window: main, groupId: null, group: null });
  const addSub = api.onDidAddPopoutGroup((popout) => {
    if (!admitPopout(popout)) watchUntilIdentified(popout);
  });
  const removeSub = api.onDidRemovePopoutGroup((popout) => {
    removePopout(popout);
  });
  const stop = () => {
    if (activeStop !== stop) return;
    activeStop = null;
    readPopouts = null;
    addSub.dispose();
    removeSub.dispose();
    for (const item of pending.values()) item.stop();
    pending.clear();
    for (const entry of [...windows.values()]) dropEntry(entry);
  };
  activeStop = stop;
  return stop;
}

/** Drop every window and the active binding. Subscriptions stay in place. */
export function resetWindowsForTests(): void {
  activeStop?.();
  mainWindow = window;
}
