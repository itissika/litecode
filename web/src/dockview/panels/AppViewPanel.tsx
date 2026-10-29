import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowClockwise,
  ArrowLeft,
  ArrowRight,
  Play,
  Prohibit,
  X,
} from "@phosphor-icons/react";
import type { IDockviewPanelProps } from "dockview-react";

import type { AppView } from "../../api/apps";
import { AppIcon } from "../../components/apps/AppIcon";
import type { AppViewState, BrowserBounds } from "../../../../desktop/src/bridge-contract";
import i18n from "../../i18n";
import { useAppsStore } from "../../stores/appsStore";
import { useToastStore } from "../../stores/toastStore";
import { movePanelToMain } from "../workbench/commands";

const ZERO_BOUNDS: BrowserBounds = { x: 0, y: 0, width: 0, height: 0 };

// A panel can be closed and reopened while its previous native view is still
// being torn down. Give every mount its own native id so a late callback from
// the old mount can never destroy the new view.
let nextNativeViewSeq = 0;

function nextNativeViewId(): string {
  nextNativeViewSeq += 1;
  return `app-${Date.now().toString(36)}-${nextNativeViewSeq.toString(36)}`;
}

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

function statusLabel(status: AppView["status"]): string {
  return i18n.t(`apps.status.${status}`, { defaultValue: status });
}

function AppUnavailable() {
  return (
    <div className="flex h-full items-center justify-center px-6 text-xs text-(--_dk-text-muted)">
      {i18n.t("apps.unavailable")}
    </div>
  );
}

function AppSurface({
  api,
  params,
}: IDockviewPanelProps<{ appId: string; url?: string }>) {
  const appId = typeof params?.appId === "string" ? params.appId : "";
  const app = useAppsStore((state) => state.apps.find((item) => item.id === appId));
  const running = app?.status === "running";
  const start = useAppsStore((state) => state.start);
  const stop = useAppsStore((state) => state.stop);
  const loadApps = useAppsStore((state) => state.load);
  const appsLoading = useAppsStore((state) => state.loading);
  const busy = useAppsStore((state) => state.busy[appId]);
  const showToast = useToastStore((state) => state.showToast);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(() => api.isVisible);
  const [createEpoch, setCreateEpoch] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const apiRef = useRef(api);
  const nativeIdRef = useRef<string | null>(null);
  if (nativeIdRef.current === null) nativeIdRef.current = nextNativeViewId();
  const nativeId = nativeIdRef.current;
  const createKeyRef = useRef<string | null>(null);
  const creatingRef = useRef(false);
  apiRef.current = api;

  const applyState = useCallback((state: AppViewState) => {
    if (state.id !== nativeIdRef.current) return;
    setCanGoBack(state.canGoBack);
    setCanGoForward(state.canGoForward);
    setLoading(state.loading);
    setError(null);
  }, []);

  const publishBounds = useCallback(() => {
    const panel = apiRef.current;
    const el = hostRef.current;
    const id = nativeIdRef.current;
    if (!id) return;
    if (!panel.isVisible || !el || panel.location.type === "popout") {
      window.litecode?.appViewSetBounds?.({ id, bounds: ZERO_BOUNDS });
      return;
    }
    const rect = el.getBoundingClientRect();
    window.litecode?.appViewSetBounds?.({
      id,
      bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    });
  }, []);

  // App views never pop out. A restored layout may still carry one in a popout
  // window; bring it home once, then the kind's canPopout flag keeps it there.
  useEffect(() => {
    if (apiRef.current.location.type === "popout") {
      movePanelToMain(apiRef.current.id);
    }
  }, [api.id]);

  useEffect(() => {
    if (!app) void loadApps();
  }, [app, loadApps]);

  useEffect(() => {
    const unsub = window.litecode?.onAppViewState?.((state) => {
      if (state.id === nativeIdRef.current) applyState(state);
    });
    return () => unsub?.();
  }, [applyState]);

  useEffect(() => {
    const panel = apiRef.current;
    const id = nativeIdRef.current;
    if (!id) return;
    const report = (active: boolean) => {
      window.litecode?.appViewSetActive?.({ id, active });
    };
    report(panel.isActive === true);
    const sub = panel.onDidActiveChange((event) => report(event.isActive));
    return () => {
      sub.dispose();
      report(false);
    };
  }, [api.id]);

  useEffect(() => {
    const panel = apiRef.current;
    const id = nativeIdRef.current;
    if (!id) return;
    const sync = (next: boolean) => {
      setVisible(next);
      window.litecode?.appViewSetVisible?.({ id, visible: next });
      if (next) requestAnimationFrame(() => publishBounds());
    };
    sync(panel.isVisible);
    const sub = panel.onDidVisibilityChange((event) => sync(event.isVisible));
    return () => {
      sub.dispose();
      sync(false);
    };
  }, [api.id, publishBounds]);

  useEffect(() => {
    if (!running) return;
    const el = hostRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => publishBounds());
    observer.observe(el);
    window.addEventListener("resize", publishBounds);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", publishBounds);
    };
  }, [publishBounds, running]);

  useEffect(() => {
    const bridge = window.litecode;
    const panel = apiRef.current;
    if (!app || app.status !== "running" || !app.url) {
      if (createKeyRef.current !== null) {
        bridge?.appViewDestroy?.(nativeId);
        createKeyRef.current = null;
        setCanGoBack(false);
        setCanGoForward(false);
      }
      setLoading(false);
      setError(null);
      return;
    }
    if (!bridge?.appViewCreate || !visible || creatingRef.current) return;
    const nextKey = `${app.id}:${app.url}`;
    if (createKeyRef.current === nextKey) return;
    if (createKeyRef.current !== null) {
      bridge.appViewDestroy?.(nativeId);
      createKeyRef.current = null;
      setCanGoBack(false);
      setCanGoForward(false);
    }
    creatingRef.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const created = await bridge.appViewCreate!({
          id: nativeId,
          appId: app.id,
          viewKey: app.view_key,
          url: app.url!,
          backgroundColor: editorBackground(),
          permissions: app.permissions,
        });
        if (cancelled) {
          bridge.appViewDestroy?.(nativeId);
          return;
        }
        createKeyRef.current = nextKey;
        applyState(created);
        bridge.appViewSetVisible?.({ id: nativeId, visible: true });
        if (panel.isActive === true) {
          bridge.appViewSetActive?.({ id: nativeId, active: true });
        }
        publishBounds();
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : i18n.t("apps.error.open"));
        }
      } finally {
        creatingRef.current = false;
        // A visibility/app change can invalidate this create while it is in
        // flight. Retry the current state once the old attempt has settled.
        if (cancelled) setCreateEpoch((value) => value + 1);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app, visible, applyState, publishBounds, createEpoch]);

  useEffect(() => {
    const bridge = window.litecode;
    return () => {
      bridge?.appViewSetVisible?.({ id: nativeId, visible: false });
      bridge?.appViewSetActive?.({ id: nativeId, active: false });
      bridge?.appViewDestroy?.(nativeId);
      createKeyRef.current = null;
    };
  }, [api.id]);

  const pageAction = (
    run: ((id: string) => Promise<AppViewState>) | undefined,
    failure: string,
  ) => {
    if (!run) return;
    const id = nativeIdRef.current;
    if (!id) return;
    void run(id).then(applyState).catch((err: unknown) => {
      setError(err instanceof Error ? err.message : failure);
    });
  };

  const onStart = async () => {
    if (!app) return;
    try {
      await start(app.id);
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err), "error");
    }
  };

  const onStop = async () => {
    if (!app) return;
    try {
      await stop(app.id);
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err), "error");
    }
  };

  const retryView = () => {
    const id = nativeIdRef.current;
    if (!id) return;
    window.litecode?.appViewDestroy?.(id);
    createKeyRef.current = null;
    setError(null);
    setLoading(false);
    setCanGoBack(false);
    setCanGoForward(false);
    setCreateEpoch((value) => value + 1);
  };

  if (!app) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-xs text-(--_dk-text-muted)">
        {appsLoading ? i18n.t("apps.loading") : i18n.t("apps.unavailable")}
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-(--_dk-editor)">
      <div className="apps-view-toolbar">
        <button
          type="button"
          className="apps-view-button"
          aria-label={i18n.t("apps.back")}
          title={i18n.t("apps.back")}
          disabled={!canGoBack}
          onClick={() =>
            pageAction(window.litecode?.appViewGoBack, i18n.t("apps.backFailed"))
          }
        >
          <ArrowLeft size={14} />
        </button>
        <button
          type="button"
          className="apps-view-button"
          aria-label={i18n.t("apps.forward")}
          title={i18n.t("apps.forward")}
          disabled={!canGoForward}
          onClick={() =>
            pageAction(window.litecode?.appViewGoForward, i18n.t("apps.forwardFailed"))
          }
        >
          <ArrowRight size={14} />
        </button>
        <button
          type="button"
          className="apps-view-button"
          aria-label={loading ? i18n.t("apps.stopLoading") : i18n.t("apps.reload")}
          title={loading ? i18n.t("apps.stopLoading") : i18n.t("apps.reload")}
          disabled={!running}
          onClick={() =>
            loading
              ? pageAction(window.litecode?.appViewStop, i18n.t("apps.stopFailed"))
              : pageAction(window.litecode?.appViewReload, i18n.t("apps.reloadFailed"))
          }
        >
          {loading ? <X size={14} /> : <ArrowClockwise size={14} />}
        </button>
        <div className="apps-view-title" title={app.name}>
          {app.name}
        </div>
        <div className="apps-view-status">
          <span className={`apps-view-dot is-${app.status}`} />
          <span className="truncate">{statusLabel(app.status)}</span>
        </div>
        {app.running ? (
          <button
            type="button"
            className="apps-view-button apps-view-stop"
            aria-label={i18n.t("apps.stop")}
            title={i18n.t("apps.stop")}
            disabled={!!busy}
            onClick={() => void onStop()}
          >
            <Prohibit size={14} />
          </button>
        ) : null}
      </div>

      {error ? (
        <div className="apps-view-error">
          <span>{error}</span>
          <button type="button" className="apps-view-retry" onClick={retryView}>
            {i18n.t("apps.retry")}
          </button>
        </div>
      ) : null}

      {!running ? (
        <div className="apps-view-card">
          <div className="apps-view-card-inner">
            <AppIcon app={app} size={40} />
            <div className={`apps-view-card-dot is-${app.status}`} />
            <div className="apps-view-card-title">
              {app.name} · {statusLabel(app.status)}
            </div>
            <p className="apps-view-card-text">
              {app.install_error?.message ?? app.runtime_error ?? i18n.t("apps.notRunning")}
            </p>
            {app.can_start ? (
              <button
                type="button"
                className="apps-action primary"
                disabled={!!busy}
                onClick={() => void onStart()}
              >
                <Play size={14} />
                {i18n.t("apps.start")}
              </button>
            ) : null}
          </div>
        </div>
      ) : (
        <div ref={hostRef} className="min-h-0 flex-1" />
      )}
    </div>
  );
}

export function AppViewPanel(
  props: IDockviewPanelProps<{ appId: string; url?: string }>,
) {
  if (typeof window.litecode?.appViewCreate !== "function") {
    return <AppUnavailable />;
  }
  return <AppSurface {...props} />;
}
