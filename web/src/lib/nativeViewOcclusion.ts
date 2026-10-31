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

const OVERLAY_ATTRIBUTE = "data-litecode-overlay";
const OVERLAY_SELECTOR = `[${OVERLAY_ATTRIBUTE}]`;
const HOST_SELECTOR = "[data-native-view-host]";
const TAB_DRAG_TARGET = ".dv-tab, .dv-void-container";
const TAB_DRAG_THRESHOLD = 4;

type OverlayPolicy = "snapshot-first" | "instant";

type OverlayEntry = {
  element: HTMLElement;
  policy: OverlayPolicy;
};

/** Small, transient surfaces must disappear from under the pointer at once. */
const INSTANT_OVERLAY_KINDS = new Set([
  "context-menu",
  "toast",
  "tooltip",
  "click-shield",
  "splash",
  "tab-drag",
]);

/** Menus owned by libraries have no hook for our own attribute. */
const THIRD_PARTY_OVERLAY_SELECTORS = [
  ".dv-context-menu",
  ".monaco-menu",
] as const;

function overlayPolicy(kind: string | null): OverlayPolicy {
  return kind && INSTANT_OVERLAY_KINDS.has(kind) ? "instant" : "snapshot-first";
}

function collectOverlays(doc: Document): OverlayEntry[] {
  const entries: OverlayEntry[] = [];
  const seen = new Set<Element>();
  for (const element of doc.querySelectorAll<HTMLElement>(OVERLAY_SELECTOR)) {
    seen.add(element);
    entries.push({
      element,
      policy: overlayPolicy(element.getAttribute(OVERLAY_ATTRIBUTE)),
    });
  }
  for (const selector of THIRD_PARTY_OVERLAY_SELECTORS) {
    for (const element of doc.querySelectorAll<HTMLElement>(selector)) {
      if (seen.has(element)) continue;
      seen.add(element);
      entries.push({ element, policy: "instant" });
    }
  }
  return entries;
}

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
  for (const { element } of collectOverlays(ctx.doc)) {
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

async function readSnapshot(
  bridge: OcclusionBridge,
  kind: NativeViewKind,
  id: string,
): Promise<string> {
  try {
    const snapshot = await bridge.nativeViewGetSnapshot({ kind, id });
    return typeof snapshot === "string" ? snapshot : "";
  } catch {
    return "";
  }
}

async function hold(
  ctx: WindowContext,
  host: HTMLElement,
  kind: NativeViewKind,
  id: string,
  key: string,
  policy: OverlayPolicy,
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
    pending: policy === "snapshot-first",
    occluded: false,
    placeholder: false,
  };
  ctx.states.set(key, state);

  if (policy === "instant") {
    // A menu must never wait behind an asynchronous snapshot.
    state.occluded = true;
    state.placeholder = true;
    useNativeViewOcclusionStore.getState().setPlaceholder(kind, id, "");
    bridge.nativeViewSetOccluded({ kind, id, occluded: true });

    void readSnapshot(bridge, kind, id).then((dataUrl) => {
      if (ctx.stopped || ctx.states.get(key)?.token !== state.token) return;
      if (dataUrl) {
        useNativeViewOcclusionStore.getState().setPlaceholder(kind, id, dataUrl);
      }
    });
    return;
  }

  const dataUrl = await readSnapshot(bridge, kind, id);

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

function createContext(
  win: Window,
  bridge: () => OcclusionBridge | undefined,
): WindowContext {
  const doc = win.document;
  const states = new Map<string, HostState>();
  let scheduled = false;
  let running = false;
  let dirty = false;

  const ctx: WindowContext = {
    win,
    doc,
    states,
    bridge,
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

    const overlays: Array<{ rect: OverlayRect; policy: OverlayPolicy }> = [];
    const nextObserved = new Set<Element>();
    for (const { element, policy } of collectOverlays(doc)) {
      const rect = elementRect(element);
      if (!rect) continue;
      overlays.push({ rect, policy });
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

      const hits = overlays.filter(({ rect: overlay }) =>
        rectsOverlap(rect, overlay),
      );
      if (hits.length > 0) {
        const policy = hits.some((hit) => hit.policy === "instant")
          ? "instant"
          : "snapshot-first";
        await hold(ctx, host, kind, id, key, policy);
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

/** A tab drag is a transient full-window overlay, not a second hide switch. */
function installTabDrag(
  win: Window,
  contexts: Map<Window, WindowContext>,
): () => void {
  let dragging = false;
  let painted = false;
  let startX = 0;
  let startY = 0;
  const overlays = new Map<Window, HTMLElement>();

  const removeOverlays = () => {
    for (const [win, overlay] of overlays) {
      overlay.remove();
      contexts.get(win)?.schedule();
    }
    overlays.clear();
  };

  const finish = () => {
    if (!dragging) return;
    dragging = false;
    painted = false;
    removeOverlays();
    win.removeEventListener("pointermove", onPointerMove, true);
    win.removeEventListener("pointerup", finish, true);
    win.removeEventListener("pointercancel", finish, true);
  };

  const paint = () => {
    if (painted) return;
    painted = true;
    for (const ctx of contexts.values()) {
      const body = ctx.doc.body;
      if (!body) continue;
      const overlay = ctx.doc.createElement("div");
      overlay.setAttribute(OVERLAY_ATTRIBUTE, "tab-drag");
      overlay.setAttribute("aria-hidden", "true");
      overlay.style.position = "fixed";
      overlay.style.top = "0";
      overlay.style.right = "0";
      overlay.style.bottom = "0";
      overlay.style.left = "0";
      overlay.style.pointerEvents = "none";
      overlay.style.background = "transparent";
      body.appendChild(overlay);
      overlays.set(ctx.win, overlay);
      ctx.schedule();
    }
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!dragging || painted) return;
    if (
      Math.abs(event.clientX - startX) + Math.abs(event.clientY - startY) <
      TAB_DRAG_THRESHOLD
    ) {
      return;
    }
    paint();
  };

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    const target = event.target as Node | null;
    if (!target || target.nodeType !== 1) return;
    if (!(target as Element).closest(TAB_DRAG_TARGET)) return;
    finish();
    dragging = true;
    startX = event.clientX;
    startY = event.clientY;
    win.addEventListener("pointermove", onPointerMove, true);
    win.addEventListener("pointerup", finish, true);
    win.addEventListener("pointercancel", finish, true);
  };

  win.addEventListener("pointerdown", onPointerDown, true);
  return () => {
    finish();
    win.removeEventListener("pointerdown", onPointerDown, true);
  };
}

/**
 * Central renderer-side manager for workbench overlays over native views.
 * Install once in the main renderer; it owns the main and popout documents
 * and sends per-view IPC through the main renderer's trusted bridge.
 */
export function installNativeViewOcclusion(): () => void {
  if (installedStop) return installedStop;

  const contexts = new Map<Window, WindowContext>();
  const mainBridge = () => bridgeOf(window);
  const stopWindows = subscribeWindows((entry) => {
    const ctx = createContext(entry.window, mainBridge);
    contexts.set(entry.window, ctx);
    const stopDrag = installTabDrag(entry.window, contexts);
    return () => {
      stopDrag();
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
