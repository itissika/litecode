import { useEffect, useMemo, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { ArrowClockwise, Info, Spinner, X } from "@phosphor-icons/react";

import type { AppCategory, AppView, RecommendationView } from "../../api/apps";
import { appsAvailable } from "../../lib/appsAvailability";
import { builtinAppName, openAppPanel, openBuiltinAppView } from "../../lib/appPanel";
import { appDescription } from "../../lib/appText";
import { Popover } from "../../components/ui/Popover";
import { useAppsStore } from "../../stores/appsStore";
import { useToastStore } from "../../stores/toastStore";
import i18n from "../../i18n";
import { AppDetail } from "../../components/apps/AppDetail";
import { RecommendationDetail } from "../../components/apps/RecommendationDetail";
import { PromptCopy } from "../../components/apps/PromptCopy";
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

const NO_RECOMMENDATIONS: RecommendationView[] = [];

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
  const progress = app.install?.progress?.percent ?? null;
  const isInstalled = app.install?.installed ?? true;
  const isBuiltin = app.source === "builtin";
  const installActive = busy === "install" && !isInstalled;
  const showSpinner =
    (!!busy && busy !== "install") || installActive || app.status === "installing";
  const showProgress =
    !!app.install?.progress ||
    app.status === "installing" ||
    app.status === "stopping" ||
    app.status === "uninstalling" ||
    installActive ||
    busy === "uninstall";

  const statusText = app.builtin_view
    ? i18n.t("apps.builtin")
    : app.running
      ? i18n.t("apps.running")
      : statusLabel(app.status);

  const renderTile = (onClick: () => void) => (
    <button
      type="button"
      className={`apps-tile is-${app.status} ${isInstalled ? "is-installed" : "is-uninstalled"}${isBuiltin ? "" : " is-external"}`}
      title={appDescription(app)}
      aria-label={`${builtinAppName(app)} — ${statusText}`}
      onClick={onClick}
      disabled={!!busy || app.status === "installing"}
    >
      <span className="apps-tile-icon">
        <AppIcon app={app} size={32} />
        {showSpinner ? (
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

function RecommendationTile({ rec }: { rec: RecommendationView }) {
  return (
    <div className="apps-tile-wrap">
      <Popover
        width={320}
        estimatedHeight={360}
        placement="down-right"
        className="block w-full"
        panelClassName="apps-popover"
        trigger={({ toggle }) => (
          <button
            type="button"
            className="apps-tile is-uninstalled is-external"
            title={appDescription(rec)}
            aria-label={`${rec.name} — ${i18n.t("apps.externalRecommendation")}`}
            onClick={toggle}
          >
            <span className="apps-tile-icon">
              <AppIcon app={rec} size={32} />
            </span>
            <span className="apps-tile-text">
              <span className="apps-tile-name">{rec.name}</span>
            </span>
          </button>
        )}
      >
        {({ close }) => <RecommendationDetail rec={rec} close={close} />}
      </Popover>
    </div>
  );
}

function RegisterInfo({ close }: { close: () => void }) {
  return (
    <div className="apps-detail">
      <div className="apps-detail-head">
        <div className="min-w-0 flex-1">
          <div className="apps-detail-name">{i18n.t("apps.installableTitle")}</div>
        </div>
        <button
          type="button"
          className="apps-icon-button"
          aria-label={i18n.t("apps.close")}
          onClick={close}
        >
          <X size={14} />
        </button>
      </div>
      <PromptCopy
        hint={i18n.t("apps.installableHint")}
        text={i18n.t("apps.installablePrompt")}
      />
    </div>
  );
}

type AvailableItem =
  | { kind: "app"; app: AppView }
  | { kind: "recommendation"; rec: RecommendationView };

function itemName(item: AvailableItem): string {
  return item.kind === "app" ? item.app.name : item.rec.name;
}

export function AppsPanel(_props: IDockviewPanelProps) {
  const [available] = useState(() => appsAvailable());
  const apps = useAppsStore((state) => state.apps);
  const recommendations = useAppsStore(
    (state) => state.snapshot?.recommendations ?? NO_RECOMMENDATIONS,
  );
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
    const refresh = () => {
      if (document.visibilityState === "visible") void load();
    };
    const timer = window.setInterval(refresh, 2500);
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [available, load]);

  // Installed is a flat list: bundled apps that LiteCode installed, plus
  // external apps the user has registered.
  const installed = useMemo(
    () =>
      apps
        .filter((app) => app.install?.installed ?? true)
        .sort((a, b) => a.name.localeCompare(b.name)),
    [apps],
  );

  // Not installed holds bundled apps that are not installed yet and external
  // recommendations that are not registered yet. Both group by purpose.
  const availableGroups = useMemo(() => {
    const groups = new Map<AppCategory, AvailableItem[]>();
    const push = (category: AppCategory, item: AvailableItem) => {
      const list = groups.get(category) ?? [];
      list.push(item);
      groups.set(category, list);
    };
    for (const app of apps) {
      if (app.install?.installed ?? true) continue;
      push(app.category, { kind: "app", app });
    }
    for (const rec of recommendations) {
      push(rec.category, { kind: "recommendation", rec });
    }
    for (const list of groups.values()) {
      list.sort((a, b) => itemName(a).localeCompare(itemName(b)));
    }
    return groups;
  }, [apps, recommendations]);

  const availableCount = useMemo(
    () => [...availableGroups.values()].reduce((sum, list) => sum + list.length, 0),
    [availableGroups],
  );

  const busyFor = (id: string) => busy[id];

  const handleOpen = async (app: AppView) => {
    if (app.builtin_view) {
      openBuiltinAppView(app.builtin_view);
      return;
    }
    if (app.running) {
      openAppPanel(app);
      return;
    }
    if (!app.can_start) {
      const managed = app.install;
      showToast(
        managed && !managed.installed
          ? i18n.t("apps.installFirst")
          : i18n.t("apps.cannotStart"),
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
          <Popover
            width={300}
            estimatedHeight={170}
            placement="down-right"
            panelClassName="apps-popover"
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className={`apps-icon-button ${open ? "is-open" : ""}`}
                aria-label={i18n.t("apps.installableTitle")}
                title={i18n.t("apps.installableTitle")}
                onClick={toggle}
              >
                <Info size={14} />
              </button>
            )}
          >
            {({ close }) => <RegisterInfo close={close} />}
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
              {entry.severity === "warning" ? (
                <span>({i18n.t("apps.registryWarning")}) </span>
              ) : null}
              <code>{entry.file}</code>
              {entry.app_id ? <span> ({entry.app_id})</span> : null}: {entry.message}
            </div>
          ))}
        </details>
      ) : null}
      {apps.length === 0 && recommendations.length === 0 && !loading ? (
        <div className="apps-empty">{i18n.t("apps.empty")}</div>
      ) : null}

      {installed.length > 0 ? (
        <section className="apps-section">
          <div className="apps-section-title">
            {i18n.t("apps.installedSection")}
            <span className="apps-count">{installed.length}</span>
          </div>
          <div className="apps-grid">
            {installed.map((app) => (
              <AppTile
                key={app.id}
                app={app}
                busy={busyFor(app.id)}
                onOpen={() => void handleOpen(app)}
              />
            ))}
          </div>
        </section>
      ) : apps.length > 0 || recommendations.length > 0 ? (
        <div className="apps-empty apps-empty-installed">
          {i18n.t("apps.emptyInstalled")}
        </div>
      ) : null}

      {availableCount > 0 ? (
        <section className="apps-section">
          <div className="apps-section-title">{i18n.t("apps.availableSection")}</div>
          {CATEGORY_ORDER.map((category) => {
            const items = availableGroups.get(category);
            if (!items || items.length === 0) return null;
            return (
              <div key={category} className="apps-category">
                <div className="apps-category-title">{categoryLabel(category)}</div>
                <div className="apps-grid">
                  {items.map((item) =>
                    item.kind === "app" ? (
                      <AppTile
                        key={`app-${item.app.id}`}
                        app={item.app}
                        busy={busyFor(item.app.id)}
                        onOpen={() => undefined}
                        detailTrigger
                      />
                    ) : (
                      <RecommendationTile key={`rec-${item.rec.id}`} rec={item.rec} />
                    ),
                  )}
                </div>
              </div>
            );
          })}
        </section>
      ) : null}
    </div>
  );
}
