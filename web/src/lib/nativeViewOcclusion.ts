import type { LitecodeDesktopBridge } from "../api/auth";
import { subscribeWindows } from "../dockview/workbench/windows";
import { useNativeViewOcclusionStore } from "../stores/nativeViewOcclusionStore";
import {
  elementRect,
  nativeViewKey,
  rectsOverlap,
  type NativeViewKind,
  type OverlayRect,
} from "./overlayGeometry";

type OcclusionBridge = Pick<
  LitecodeDesktopBridge,
  "nativeViewGetSnapshot" | "nativeViewSetOccluded"
>;

type HostState = {
  kind: NativeViewKind;
  id: string;
  key: string;
  token: number;
  pending: boolean;
  occluded: boolean;
  placeholder: boolean;
};

type WindowContext = {
  win: Window;
  doc: Document;
  states: Map<string, HostState>;
  bridge: () => OcclusionBridge | undefined;
  schedule: () => void;
  stop: () => void;
  stopped: boolean;
};

const OVERLAY_SELECTOR = "[data-litecode-overlay]";
const HOST_SELECTOR = "[data-native-view-host]";

let installedStop: (() => void) | null = null;
let tokenSeq = 0;

function bridgeOf(win: Window): OcclusionBridge | undefined {
  const bridge = (win as Window & { litecode?: OcclusionBridge }).litecode;
  if (
    !bridge ||
    typeof bridge.nativeViewGetSnapshot !== "function" ||
    typeof bridge.nativeViewSetOccluded !== "function"
  ) {
    return undefined;
  }
  return bridge;
}

function nextFrame(win: Window): Promise<void> {
  return new Promise((resolve) => {
    const raf = win.requestAnimationFrame;
    if (typeof raf === "function") {
      raf(() => resolve());
      return;
    }
    win.setTimeout(resolve, 0);
  });
}

function stillCovered(ctx: WindowContext, host: HTMLElement): boolean {
  if (!host.isConnected) return false;
  const hostRect = elementRect(host);
  if (!hostRect) return false;
  for (const element of ctx.doc.querySelectorAll<HTMLElement>(OVERLAY_SELECTOR)) {
    const rect = elementRect(element);
    if (rect && rectsOverlap(hostRect, rect)) return true;
  }
  return false;
}

function release(ctx: WindowContext, key: string): void {
  const state = ctx.states.get(key);
  if (!state) return;
  ctx.states.delete(key);
  if (state.occluded) {
    ctx.bridge()?.nativeViewSetOccluded({
      kind: state.kind,
      id: state.id,
      occluded: false,
    });
  }
  if (!state.placeholder) return;

  const clear = () => {
    const current = ctx.states.get(key);
    if (current && current.token !== state.token) return;
    useNativeViewOcclusionStore
      .getState()
      .clearPlaceholder(state.kind, state.id);
  };
  if (ctx.stopped) {
    clear();
    return;
  }
  // The IPC show is asynchronous; keep the still frame for one task so the
  // native layer is back before the DOM placeholder disappears.
  void nextFrame(ctx.win).then(clear);
}

async function hold(
  ctx: WindowContext,
  host: HTMLElement,
  kind: NativeViewKind,
  id: string,
  key: string,
): Promise<void> {
  const existing = ctx.states.get(key);
  if (existing?.occluded || existing?.pending) return;

  const bridge = ctx.bridge();
  if (!bridge) return;

  const state: HostState = {
    kind,
    id,
    key,
    token: ++tokenSeq,
    pending: true,
    occluded: false,
    placeholder: false,
  };
  ctx.states.set(key, state);

  let dataUrl = "";
  try {
    const snapshot = await bridge.nativeViewGetSnapshot({ kind, id });
    dataUrl = typeof snapshot === "string" ? snapshot : "";
  } catch {
    dataUrl = "";
  }

  if (ctx.stopped || ctx.states.get(key)?.token !== state.token) return;
  if (!stillCovered(ctx, host)) {
    release(ctx, key);
    return;
  }

  useNativeViewOcclusionStore.getState().setPlaceholder(kind, id, dataUrl);
  state.placeholder = true;

  // Let React commit the placeholder before the native layer disappears.
  await nextFrame(ctx.win);
  const current = ctx.states.get(key);
  if (ctx.stopped || current?.token !== state.token) {
    // A newer hold owns the key and may already have installed its own frame.
    if (!current) {
      useNativeViewOcclusionStore.getState().clearPlaceholder(kind, id);
    }
    return;
  }

  state.pending = false;
  state.occluded = true;
  bridge.nativeViewSetOccluded({ kind, id, occluded: true });
}

function overlapsAny(hostRect: OverlayRect, overlays: OverlayRect[]): boolean {
  return overlays.some((overlay) => rectsOverlap(hostRect, overlay));
}

function createContext(win: Window): WindowContext {
  const doc = win.document;
  const states = new Map<string, HostState>();
  let scheduled = false;
  let running = false;
  let dirty = false;

  const ctx: WindowContext = {
    win,
    doc,
    states,
    bridge: () => bridgeOf(win),
    stopped: false,
    schedule: () => undefined,
    stop: () => undefined,
  };

  const observed = new Set<Element>();
  const MutationObserverCtor = (
    win as Window & { MutationObserver?: typeof MutationObserver }
  ).MutationObserver;
  const mutationObserver = MutationObserverCtor
    ? new MutationObserverCtor(() => ctx.schedule())
    : null;
  const ResizeObserverCtor = (
    win as Window & { ResizeObserver?: typeof ResizeObserver }
  ).ResizeObserver;
  const resizeObserver = ResizeObserverCtor
    ? new ResizeObserverCtor(() => ctx.schedule())
    : null;

  const observe = (element: Element, next: Set<Element>) => {
    next.add(element);
    if (observed.has(element) || !resizeObserver) return;
    observed.add(element);
    resizeObserver.observe(element);
  };

  const reconcile = async (): Promise<void> => {
    if (ctx.stopped || !ctx.bridge()) return;

    const overlays: OverlayRect[] = [];
    const nextObserved = new Set<Element>();
    for (const element of doc.querySelectorAll<HTMLElement>(OVERLAY_SELECTOR)) {
      const rect = elementRect(element);
      if (!rect) continue;
      overlays.push(rect);
      observe(element, nextObserved);
    }

    const seen = new Set<string>();
    for (const host of doc.querySelectorAll<HTMLElement>(HOST_SELECTOR)) {
      const kind = host.dataset.nativeViewKind;
      const id = host.dataset.nativeViewId;
      if ((kind !== "browser" && kind !== "app") || !id) continue;

      const key = nativeViewKey(kind, id);
      seen.add(key);
      observe(host, nextObserved);

      const rect = elementRect(host);
      if (!rect) {
        release(ctx, key);
        continue;
      }

      if (overlapsAny(rect, overlays)) {
        await hold(ctx, host, kind, id, key);
      } else {
        release(ctx, key);
      }
    }

    for (const key of [...states.keys()]) {
      if (!seen.has(key)) release(ctx, key);
    }

    if (resizeObserver) {
      for (const element of observed) {
        if (nextObserved.has(element)) continue;
        resizeObserver.unobserve(element);
        observed.delete(element);
      }
    }
  };

  const run = async (): Promise<void> => {
    if (ctx.stopped) return;
    if (running) {
      dirty = true;
      return;
    }
    running = true;
    try {
      do {
        dirty = false;
        await reconcile();
      } while (dirty && !ctx.stopped);
    } finally {
      running = false;
      if (dirty && !ctx.stopped) ctx.schedule();
    }
  };

  const schedule = () => {
    if (scheduled || ctx.stopped) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      void run().catch(() => undefined);
    });
  };
  ctx.schedule = schedule;

  const onScroll = () => schedule();
  const onResize = () => schedule();
  const onVisibilityChange = () => schedule();
  win.addEventListener("scroll", onScroll, true);
  win.addEventListener("resize", onResize);
  doc.addEventListener("visibilitychange", onVisibilityChange);

  if (mutationObserver && doc.body) {
    mutationObserver.observe(doc.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: [
        "data-litecode-overlay",
        "class",
        "style",
        "hidden",
        "aria-hidden",
      ],
    });
  }

  ctx.stop = () => {
    if (ctx.stopped) return;
    ctx.stopped = true;
    for (const state of [...states.values()]) release(ctx, state.key);
    mutationObserver?.disconnect();
    resizeObserver?.disconnect();
    observed.clear();
    win.removeEventListener("scroll", onScroll, true);
    win.removeEventListener("resize", onResize);
    doc.removeEventListener("visibilitychange", onVisibilityChange);
  };

  schedule();
  return ctx;
}

/**
 * Central renderer-side manager for workbench overlays over native views.
 * Install once in the main renderer. Phase 1 is main-window only: popout
 * renderers cannot send trusted IPC, so they stay on the old chrome lease.
 */
export function installNativeViewOcclusion(): () => void {
  if (installedStop) return installedStop;

  const contexts = new Map<Window, WindowContext>();
  const stopWindows = subscribeWindows((entry) => {
    // Popout renderers are deliberately not trusted for IPC. Phase 1 keeps
    // them on the old chrome lease; the main window owns this manager.
    if (entry.dockId !== null) return () => undefined;
    const ctx = createContext(entry.window);
    contexts.set(entry.window, ctx);
    return () => {
      ctx.stop();
      contexts.delete(entry.window);
    };
  });

  const stop = () => {
    for (const ctx of contexts.values()) ctx.stop();
    contexts.clear();
    stopWindows();
    installedStop = null;
  };
  installedStop = stop;
  return stop;
}

export function resetNativeViewOcclusionForTests(): void {
  installedStop?.();
  installedStop = null;
  useNativeViewOcclusionStore.getState().clearAll();
}
