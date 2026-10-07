import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { ArrowLeft, ArrowRight } from "@phosphor-icons/react";
import type { IDockviewPanelProps } from "dockview-react";

import {
  browserBoundsPlace,
  displayBrowserUrl,
  MAIN_BROWSER_PLACE,
  normalizeBrowserUrl,
  sameBrowserUrl,
  type BrowserBounds,
  type BrowserPanelState,
} from "../../lib/browserPanel";
import { dockIdFromLocation } from "../popout/location";

const ZERO_BOUNDS: BrowserBounds = { x: 0, y: 0, width: 0, height: 0 };

function editorBackground(): string {
  const token = getComputedStyle(document.documentElement)
    .getPropertyValue("--_dk-editor")
    .trim();
  if (!token) return "#0a0a0a";
  const probe = document.createElement("div");
  probe.style.position = "absolute";
  probe.style.left = "-9999px";
  probe.style.pointerEvents = "none";
  probe.style.backgroundColor = token;
  document.body.appendChild(probe);
  const resolved = getComputedStyle(probe).backgroundColor;
  probe.remove();
  if (!resolved || resolved === "rgba(0, 0, 0, 0)") return "#0a0a0a";
  return resolved;
}

function BrowserUnavailable() {
  return (
    <div className="flex h-full items-center justify-center px-6 text-xs text-(--_dk-text-muted)">
      Browser is available in the desktop app.
    </div>
  );
}

function BrowserSurface({ api, params }: IDockviewPanelProps<{ url?: string }>) {
  const initialUrl = typeof params?.url === "string" ? params.url : "";
  const [address, setAddress] = useState(() => displayBrowserUrl(initialUrl));
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const placeRef = useRef(MAIN_BROWSER_PLACE);
  const focusedRef = useRef(false);
  const latestUrlRef = useRef(displayBrowserUrl(initialUrl));
  const apiRef = useRef(api);
  const paramsRef = useRef(params);
  apiRef.current = api;
  paramsRef.current = params;

  const applyState = useCallback((state: BrowserPanelState) => {
    setCanGoBack(state.canGoBack);
    setCanGoForward(state.canGoForward);
    setLoading(state.loading);
    setError(null);
    const shown = displayBrowserUrl(state.url);
    latestUrlRef.current = shown;
    if (!focusedRef.current) setAddress(shown);
    const panel = apiRef.current;
    const title = state.title.trim() || "Browser";
    if (panel.title !== title) panel.setTitle(title);
    const current = panel.getParameters<{ url?: string }>();
    const currentUrl = typeof current?.url === "string" ? current.url : "";
    if (
      state.url &&
      state.url !== "about:blank" &&
      !sameBrowserUrl(state.url, currentUrl)
    ) {
      panel.updateParameters({ url: state.url });
    }
  }, []);

  const publishBounds = useCallback(() => {
    const panel = apiRef.current;
    const el = hostRef.current;
    const send = (bounds: BrowserBounds, place: string) => {
      window.litecode?.browserSetBounds?.({ id: panel.id, bounds, place });
    };
    // Hidden dockview panels stay laid out with visibility:hidden, so their
    // box is still non-zero. A zero rect drops the native view until the
    // panel is actually showing. CSS pixels match Electron DIP at zoom 1.
    // A rectangle is sent only when the host element and the native view are
    // in the same window. Popout coordinates must not land on the main window.
    if (!panel.isVisible || !el) {
      send(ZERO_BOUNDS, placeRef.current);
      return;
    }
    const place = browserBoundsPlace(
      el.ownerDocument === document,
      placeRef.current,
    );
    if (!place) {
      send(ZERO_BOUNDS, MAIN_BROWSER_PLACE);
      return;
    }
    const rect = el.getBoundingClientRect();
    send(
      { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      place,
    );
  }, []);

  useEffect(() => {
    const unsub = window.litecode?.onBrowserState?.((state) => {
      if (state.id === apiRef.current.id) applyState(state);
    });
    return () => unsub?.();
  }, [applyState]);

  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => publishBounds());
    observer.observe(el);
    window.addEventListener("resize", publishBounds);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", publishBounds);
    };
  }, [publishBounds]);

  useEffect(() => {
    let cancelled = false;
    const bridge = window.litecode;
    const panel = apiRef.current;
    if (!bridge?.browserCreate) return;
    const saved =
      typeof paramsRef.current?.url === "string" ? paramsRef.current.url : "";
    void (async () => {
      try {
        const created = await bridge.browserCreate!({
          id: panel.id,
          backgroundColor: editorBackground(),
        });
        if (cancelled) return;
        applyState(created);
        const normalized = normalizeBrowserUrl(saved);
        if (normalized && !sameBrowserUrl(normalized, created.url)) {
          const next = await bridge.browserNavigate!({ id: panel.id, url: normalized });
          if (!cancelled) applyState(next);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Could not open the page");
        }
      }
      if (cancelled) return;
      bridge.browserSetVisible?.({ id: panel.id, visible: panel.isVisible });
      publishBounds();
    })();
    return () => {
      cancelled = true;
    };
  }, [api.id, applyState, publishBounds]);

  useEffect(() => {
    const panel = apiRef.current;
    const sync = (visible: boolean) => {
      window.litecode?.browserSetVisible?.({ id: panel.id, visible });
      if (visible) requestAnimationFrame(() => publishBounds());
    };
    sync(panel.isVisible);
    const sub = panel.onDidVisibilityChange((event) => sync(event.isVisible));
    return () => {
      sub.dispose();
      sync(false);
    };
  }, [api.id, publishBounds]);

  useEffect(() => {
    const panel = apiRef.current;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stop = () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
    };
    const hideOnMain = () => {
      window.litecode?.browserSetBounds?.({
        id: panel.id,
        bounds: ZERO_BOUNDS,
        place: MAIN_BROWSER_PLACE,
      });
    };
    const aimPopout = (attempt: number) => {
      if (disposed) return;
      const location = panel.location;
      if (location.type !== "popout") return;
      placeRef.current = MAIN_BROWSER_PLACE;
      hideOnMain();
      const dock = dockIdFromLocation(location);
      const attached =
        !!dock &&
        window.litecode?.browserSetHost?.({ id: panel.id, popoutId: dock }) === true;
      if (attached && dock) {
        placeRef.current = dock;
        publishBounds();
        requestAnimationFrame(() => {
          if (!disposed && placeRef.current === dock) publishBounds();
        });
        return;
      }
      if (attempt < 10) {
        timer = setTimeout(() => aimPopout(attempt + 1), 50);
      }
    };
    const aimMain = () => {
      stop();
      placeRef.current = MAIN_BROWSER_PLACE;
      hideOnMain();
      window.litecode?.browserSetHost?.({ id: panel.id, popoutId: null });
      publishBounds();
      requestAnimationFrame(() => {
        if (!disposed && placeRef.current === MAIN_BROWSER_PLACE) publishBounds();
      });
    };
    if (panel.location.type === "popout") aimPopout(0);
    const sub = panel.onDidLocationChange((event) => {
      if (event.location.type === "popout") aimPopout(0);
      else aimMain();
    });
    return () => {
      disposed = true;
      stop();
      sub.dispose();
    };
  }, [api.id, publishBounds]);

  const go = (direction: "back" | "forward") => {
    const bridge = window.litecode;
    const run =
      direction === "back" ? bridge?.browserGoBack : bridge?.browserGoForward;
    if (!run) return;
    void run(api.id).then(applyState).catch((err: unknown) => {
      setError(err instanceof Error ? err.message : "Could not move through history");
    });
  };

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    const url = normalizeBrowserUrl(address);
    if (!url) {
      setError("Enter an http(s) address");
      return;
    }
    setError(null);
    void window.litecode
      ?.browserNavigate?.({ id: api.id, url })
      .then(applyState)
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : "Could not open the page");
      });
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-(--_dk-editor)">
      <form
        className="flex h-8 shrink-0 items-center gap-1 border-b border-(--_dk-border-visible) px-1.5"
        onSubmit={onSubmit}
      >
        <button
          type="button"
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-(--_dk-text-muted) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary) disabled:pointer-events-none disabled:opacity-40"
          aria-label="Back"
          title="Back"
          disabled={!canGoBack}
          onClick={() => go("back")}
        >
          <ArrowLeft size={14} />
        </button>
        <button
          type="button"
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-(--_dk-text-muted) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary) disabled:pointer-events-none disabled:opacity-40"
          aria-label="Forward"
          title="Forward"
          disabled={!canGoForward}
          onClick={() => go("forward")}
        >
          <ArrowRight size={14} />
        </button>
        <input
          aria-label="Address"
          aria-busy={loading}
          spellCheck={false}
          className="h-6 min-w-0 flex-1 rounded border border-(--_dk-border-visible) bg-(--_dk-root) px-2 text-xs text-(--_dk-text-secondary) outline-none focus:border-(--_dk-text-muted)"
          value={address}
          placeholder="Enter a URL"
          onChange={(event) => setAddress(event.target.value)}
          onFocus={() => {
            focusedRef.current = true;
          }}
          onBlur={() => {
            focusedRef.current = false;
            setAddress(latestUrlRef.current);
          }}
        />
      </form>
      {error ? (
        <p className="shrink-0 px-2 py-1 text-xs text-(--_dk-amber-500)">{error}</p>
      ) : null}
      <div ref={hostRef} className="min-h-0 flex-1" />
    </div>
  );
}

export function BrowserPanel(props: IDockviewPanelProps<{ url?: string }>) {
  if (typeof window.litecode?.browserCreate !== "function") {
    return <BrowserUnavailable />;
  }
  return <BrowserSurface {...props} />;
}
