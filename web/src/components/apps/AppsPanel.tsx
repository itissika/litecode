import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { ArrowClockwise, Info, Spinner } from "@phosphor-icons/react";

import type { AppCategory, AppView } from "../../api/apps";
import { appsAvailable } from "../../lib/appsAvailability";
import { formatBytes } from "../../lib/bytes";
import { builtinAppName, openAppPanel, openBuiltinAppView } from "../../lib/appPanel";
import { appDescription } from "../../lib/appText";
import { pushBrowserObscure } from "../../lib/browserObscure";
import { Popover } from "../../components/ui/Popover";
import { useAppsStore } from "../../stores/appsStore";
import { useToastStore } from "../../stores/toastStore";
import i18n from "../../i18n";
import { AppDetail } from "../../components/apps/AppDetail";
import { AppIcon } from "../../components/apps/AppIcon";

const CATEGORY_ORDER: AppCategory[] = [
  "data",
  "analysis",
  "document",
  "design",
  "image",
  "video",
  "audio",
  "task",
  "general",
];

function categoryLabel(category: AppCategory): string {
  return i18n.t(`apps.category.${category}`, { defaultValue: category });
}

function statusLabel(status: AppView["status"]): string {
  return i18n.t(`apps.status.${status}`, { defaultValue: status });
}

function AppTile({
  app,
  busy,
  onOpen,
  detailTrigger,
}: {
  app: AppView;
  busy?: string;
  onOpen: () => void;
  detailTrigger?: boolean;
}) {
  const progress = app.install_progress?.percent ?? null;
  const showProgress =
    app.status === "installing" ||
    app.status === "stopping" ||
    app.status === "uninstalling" ||
    busy === "install" ||
    busy === "uninstall";

  const statusText = app.builtin_view
    ? i18n.t("apps.builtin")
    : app.running
      ? i18n.t("apps.running")
      : statusLabel(app.status);

  const renderTile = (onClick: () => void) => (
    <button
      type="button"
      className={`apps-tile is-${app.status} ${app.installed ? "is-installed" : "is-uninstalled"}`}
      title={appDescription(app)}
      aria-label={`${builtinAppName(app)} — ${statusText}`}
      onClick={onClick}
      disabled={!!busy || app.status === "installing"}
    >
      <span className="apps-tile-icon">
        <AppIcon app={app} size={32} />
        {busy || app.status === "installing" ? (
          <span className="apps-tile-spinner">
            <Spinner size={12} className="apps-spin" />
          </span>
        ) : null}
      </span>
      <span className="apps-tile-text">
        <span className="apps-tile-name">{builtinAppName(app)}</span>
      </span>
      {showProgress ? (
        <span className="apps-tile-progress">
          <span
            className={progress === null ? "is-indeterminate" : ""}
            style={progress === null ? undefined : { width: `${Math.max(4, progress)}%` }}
          />
        </span>
      ) : null}
    </button>
  );

  if (detailTrigger) {
    return (
      <Popover
        width={320}
        estimatedHeight={420}
        placement="down-right"
        className="block w-full"
        panelClassName="apps-popover"
        trigger={({ toggle }) => renderTile(toggle)}
      >
        {({ close }) => <AppDetail app={app} close={close} />}
      </Popover>
    );
  }

  return (
    <div className="apps-tile-wrap">
      {renderTile(onOpen)}
      <Popover
        width={320}
        estimatedHeight={420}
        placement="down-right"
        className="apps-info-wrap"
        panelClassName="apps-popover"
        trigger={({ open, toggle }) => (
          <button
            type="button"
            className={`apps-info-button ${open ? "is-open" : ""}`}
            aria-label={i18n.t("apps.details")}
            title={i18n.t("apps.details")}
            onClick={(event) => {
              event.stopPropagation();
              toggle();
            }}
          >
            <Info size={12} />
          </button>
        )}
      >
        {({ close }) => <AppDetail app={app} close={close} />}
      </Popover>
    </div>
  );
}

function RegisterInfo({
  close,
  paths,
}: {
  close: () => void;
  paths: { global: string; workspace: string } | null | undefined;
}) {
  useLayoutEffect(() => pushBrowserObscure(), []);
  return (
    <div className="apps-register">
      <div className="apps-register-title">{i18n.t("apps.registerTitle")}</div>
      <p>{i18n.t("apps.registerBody")}</p>
      <div className="apps-register-paths">
        <div>
          <span>{i18n.t("apps.registerWorkspace")}</span>
          <code>{paths?.workspace ?? "—"}</code>
        </div>
        <div>
          <span>{i18n.t("apps.registerGlobal")}</span>
          <code>{paths?.global ?? "—"}</code>
        </div>
      </div>
      <pre className="apps-register-prompt">{i18n.t("apps.registerPrompt")}</pre>
      <button type="button" className="apps-action" onClick={close}>
        {i18n.t("apps.close")}
      </button>
    </div>
  );
}

function AppGrid({
  apps,
  busyFor,
  onOpen,
  detailMode,
}: {
  apps: AppView[];
  busyFor: (id: string) => string | undefined;
  onOpen: (app: AppView) => void;
  detailMode: "tile" | "info";
}) {
  return (
    <div className="apps-grid">
      {apps.map((app) => (
        <AppTile
          key={app.id}
          app={app}
          busy={busyFor(app.id)}
          onOpen={() => onOpen(app)}
          detailTrigger={detailMode === "tile" && !app.installed}
        />
      ))}
    </div>
  );
}

export function AppsPanel(_props: IDockviewPanelProps) {
  const [available] = useState(() => appsAvailable());
  const apps = useAppsStore((state) => state.apps);
  const loading = useAppsStore((state) => state.loading);
  const error = useAppsStore((state) => state.error);
  const busy = useAppsStore((state) => state.busy);
  const snapshot = useAppsStore((state) => state.snapshot);
  const load = useAppsStore((state) => state.load);
  const start = useAppsStore((state) => state.start);
  const showToast = useToastStore((state) => state.showToast);

  useEffect(() => {
    if (!available) return;
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 2500);
    return () => window.clearInterval(timer);
  }, [available, load]);

  const installed = useMemo(
    () =>
      apps
        .filter((app) => app.installed)
        .sort((a, b) => a.name.localeCompare(b.name)),
    [apps],
  );
  const uninstalled = useMemo(() => {
    const groups = new Map<AppCategory, AppView[]>();
    for (const app of apps) {
      if (app.installed) continue;
      const list = groups.get(app.category) ?? [];
      list.push(app);
      groups.set(app.category, list);
    }
    return CATEGORY_ORDER.flatMap((category) => {
      const list = (groups.get(category) ?? []).sort((a, b) =>
        a.name.localeCompare(b.name),
      );
      return list.map((app) => ({ app, category }));
    });
  }, [apps]);

  const busyFor = (id: string) => busy[id];

  const handleOpen = async (app: AppView) => {
    if (app.builtin_view) {
      openBuiltinAppView(app.builtin_view);
      return;
    }
    if (app.status === "crashed" || app.status === "error") {
      showToast(i18n.t("apps.cannotStart"), "info");
      return;
    }
    if (app.running && app.status === "running") {
      openAppPanel(app);
      return;
    }
    if (!app.installed || !app.can_start) {
      showToast(
        app.installed
          ? i18n.t("apps.cannotStart")
          : i18n.t("apps.installFirst"),
        "info",
      );
      return;
    }
    try {
      await start(app.id);
      const next = useAppsStore.getState().apps.find((item) => item.id === app.id);
      if (next?.running) openAppPanel(next);
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err), "error");
    }
  };

  if (!available) {
    return (
      <div className="apps-panel apps-empty">
        <p>{i18n.t("apps.remoteUnavailable")}</p>
      </div>
    );
  }

  return (
    <div className="apps-panel">
      <div className="apps-panel-head">
        <div className="apps-panel-title">
          <span>{i18n.t("apps.title")}</span>
          {loading ? <Spinner size={12} className="apps-spin" /> : null}
        </div>
        <div className="apps-panel-head-right">
          <span className="apps-space" title={i18n.t("apps.availableBytes")}>
            {formatBytes(snapshot?.available_bytes)}
          </span>
          <Popover
            width={300}
            estimatedHeight={180}
            placement="down-right"
            panelClassName="apps-popover"
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className={`apps-icon-button ${open ? "is-open" : ""}`}
                aria-label={i18n.t("apps.registerHint")}
                title={i18n.t("apps.registerHint")}
                onClick={toggle}
              >
                <Info size={14} />
              </button>
            )}
          >
            {({ close }) => (
              <RegisterInfo close={close} paths={snapshot?.registry_paths} />
            )}
          </Popover>
          <button
            type="button"
            className="apps-icon-button"
            aria-label={i18n.t("apps.refresh")}
            title={i18n.t("apps.refresh")}
            onClick={() => void load()}
          >
            <ArrowClockwise size={14} />
          </button>
        </div>
      </div>

      {error ? <div className="apps-panel-error">{error}</div> : null}
      {snapshot?.registry_errors?.length ? (
        <details className="apps-registry-errors">
          <summary>
            {i18n.t("apps.registryErrors", {
              count: snapshot.registry_errors.length,
            })}
          </summary>
          {snapshot.registry_errors.map((entry, index) => (
            <div key={`${entry.file}-${index}`}>
              <strong>{entry.source}</strong>{" "}
              <code>{entry.file}</code>
              {entry.app_id ? <span> ({entry.app_id})</span> : null}: {entry.message}
            </div>
          ))}
        </details>
      ) : null}
      {apps.length === 0 && !loading ? (
        <div className="apps-empty">{i18n.t("apps.empty")}</div>
      ) : null}

      {installed.length > 0 ? (
        <section className="apps-section">
          <div className="apps-section-title">
            {i18n.t("apps.installedSection")}
            <span className="apps-count">{installed.length}</span>
          </div>
          <AppGrid
            apps={installed}
            busyFor={busyFor}
            onOpen={(app) => void handleOpen(app)}
            detailMode="info"
          />
        </section>
      ) : apps.length > 0 ? (
        <div className="apps-empty apps-empty-installed">
          {i18n.t("apps.emptyInstalled")}
        </div>
      ) : null}

      {uninstalled.length > 0 ? (
        <section className="apps-section">
          <div className="apps-section-title">{i18n.t("apps.availableSection")}</div>
          {CATEGORY_ORDER.map((category) => {
            const list = uninstalled
              .filter((entry) => entry.category === category)
              .map((entry) => entry.app);
            if (list.length === 0) return null;
            return (
              <div key={category} className="apps-category">
                <div className="apps-category-title">{categoryLabel(category)}</div>
                <AppGrid
                  apps={list}
                  busyFor={busyFor}
                  onOpen={() => undefined}
                  detailMode="tile"
                />
              </div>
            );
          })}
        </section>
      ) : null}

    </div>
  );
}
