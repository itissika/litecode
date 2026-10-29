import { useEffect, useLayoutEffect, useState } from "react";

import {
  ArrowClockwise,
  ArrowSquareOut,
  DownloadSimple,
  Play,
  Prohibit,
  Trash,
  X,
} from "@phosphor-icons/react";

import { fetchAppReadme, type AppView } from "../../api/apps";
import { formatBytes } from "../../lib/bytes";
import { builtinAppName, openAppPanel, openBuiltinAppView } from "../../lib/appPanel";
import { appDescription } from "../../lib/appText";
import { pushBrowserObscure } from "../../lib/browserObscure";
import i18n from "../../i18n";
import { useAppsStore } from "../../stores/appsStore";
import { useToastStore } from "../../stores/toastStore";
import { AgentMarkdown } from "../AgentMarkdown";
import { AppIcon } from "./AppIcon";

function statusLabel(status: AppView["status"]): string {
  return i18n.t(`apps.status.${status}`, { defaultValue: status });
}

function sourceLabel(source: AppView["source"]): string {
  return i18n.t(`apps.source.${source}`, { defaultValue: source });
}

function categoryLabel(category: AppView["category"]): string {
  return i18n.t(`apps.category.${category}`, { defaultValue: category });
}

export function AppDetail({
  app,
  close,
}: {
  app: AppView;
  close: () => void;
}) {
  const busy = useAppsStore((state) => state.busy[app.id]);
  const availableBytes = useAppsStore((state) => state.snapshot?.available_bytes);
  const install = useAppsStore((state) => state.install);
  const start = useAppsStore((state) => state.start);
  const stop = useAppsStore((state) => state.stop);
  const uninstall = useAppsStore((state) => state.uninstall);
  const trust = useAppsStore((state) => state.trust);
  const showToast = useToastStore((state) => state.showToast);
  const [readme, setReadme] = useState<string | null>(null);
  const [readmeError, setReadmeError] = useState(false);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const isBuiltin = !!app.builtin_view;

  useLayoutEffect(() => pushBrowserObscure(), []);

  useEffect(() => {
    setConfirmUninstall(false);
  }, [app.id]);

  useEffect(() => {
    // Bundled apps have no upstream README; their summary is shown instead.
    if (app.builtin_view) return;
    let cancelled = false;
    setReadme(null);
    setReadmeError(false);
    void fetchAppReadme(app.id)
      .then((text) => {
        if (!cancelled) setReadme(text);
      })
      .catch(() => {
        if (!cancelled) setReadmeError(true);
      });
    return () => {
      cancelled = true;
    };
  }, [app.id, app.builtin_view]);

  const action = async (task: () => Promise<void>) => {
    try {
      await task();
    } catch (error) {
      showToast(error instanceof Error ? error.message : String(error), "error");
    }
  };

  const open = () => {
    if (app.builtin_view) openBuiltinAppView(app.builtin_view);
    else openAppPanel(app);
    close();
  };

  const canInstall = app.can_install;
  const showTrust = !app.trusted && !!app.trust_preview;
  const actionLabel = app.installed
    ? app.running
      ? i18n.t("apps.open")
      : i18n.t("apps.start")
    : app.can_install || !app.trusted
      ? app.install_error
        ? i18n.t("apps.retry")
        : i18n.t("apps.install")
      : i18n.t("apps.externalInstall");
  const size = app.installed ? app.size_bytes : app.required_bytes;
  const sizeLabel =
    size === null || size === undefined
      ? i18n.t("apps.sizeUnknown")
      : app.installed
        ? i18n.t("apps.installedSize", { size: formatBytes(size) })
        : i18n.t("apps.estimatedSize", { size: formatBytes(size) });

  return (
    <div className="apps-detail" onPointerDown={(event) => event.stopPropagation()}>
      <div className="apps-detail-head">
        <AppIcon app={app} size={48} />
        <div className="min-w-0 flex-1">
          <div className="apps-detail-name">{builtinAppName(app)}</div>
          <div className="apps-detail-meta">
            <span>{categoryLabel(app.category)}</span>
            <span>·</span>
            <span>{sourceLabel(app.source)}</span>
            <span>·</span>
            <span className={`apps-status-text is-${app.status}`}>{statusLabel(app.status)}</span>
          </div>
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

      <p className="apps-detail-description">{appDescription(app)}</p>
      {isBuiltin && app.summary ? (
        <p className="apps-hint">{app.summary}</p>
      ) : null}

      <div className="apps-detail-space">
        {isBuiltin ? (
          <span>{i18n.t("apps.builtinHint")}</span>
        ) : (
          <>
            <span>{sizeLabel}</span>
            {availableBytes !== null && availableBytes !== undefined ? (
              <span>{i18n.t("apps.availableSpace", { size: formatBytes(availableBytes) })}</span>
            ) : null}
            <span>{app.data_scope === "workspace" ? i18n.t("apps.workspaceData") : i18n.t("apps.appData")}</span>
            {app.single_instance ? <span>{i18n.t("apps.singleInstance")}</span> : null}
            {!app.installed && !app.can_install && app.trusted ? (
              <span>{i18n.t("apps.externalInstall")}</span>
            ) : null}
          </>
        )}
      </div>

      <div className="apps-detail-actions">
        {isBuiltin ? (
          <button type="button" className="apps-action primary" onClick={open}>
            <ArrowSquareOut size={14} />
            {i18n.t("apps.open")}
          </button>
        ) : app.running ? (
          <>
            <button type="button" className="apps-action primary" onClick={open}>
              <ArrowSquareOut size={14} />
              {i18n.t("apps.open")}
            </button>
            <button
              type="button"
              className="apps-action"
              disabled={!!busy}
              onClick={() => void action(() => stop(app.id))}
            >
              <Prohibit size={14} />
              {i18n.t("apps.stop")}
            </button>
          </>
        ) : app.installed ? (
          <button
            type="button"
            className="apps-action primary"
            disabled={!!busy || !app.can_start}
            onClick={() => void action(() => start(app.id))}
          >
            <Play size={14} />
            {i18n.t("apps.start")}
          </button>
        ) : (
          <button
            type="button"
            className="apps-action primary"
            disabled={!!busy || !canInstall}
            onClick={() => void action(() => install(app.id))}
          >
            <DownloadSimple size={14} />
            {actionLabel}
          </button>
        )}
        {app.installed && app.can_uninstall ? (
          <button
            type="button"
            className="apps-action danger"
            disabled={!!busy}
            onClick={() => setConfirmUninstall(true)}
          >
            <Trash size={14} />
            {i18n.t("apps.uninstall")}
          </button>
        ) : null}
        {!isBuiltin && app.installed && !app.running ? (
          <button
            type="button"
            className="apps-action"
            disabled={!!busy}
            onClick={open}
          >
            <ArrowClockwise size={14} />
            {i18n.t("apps.openPanel")}
          </button>
        ) : null}
      </div>

      {confirmUninstall ? (
        <div className="apps-confirm">
          <span>{i18n.t("apps.uninstallConfirm", { name: app.name })}</span>
          <div>
            <button
              type="button"
              className="apps-action danger"
              disabled={!!busy}
              onClick={() => {
                setConfirmUninstall(false);
                void action(() => uninstall(app.id));
              }}
            >
              {i18n.t("apps.uninstallConfirmAction")}
            </button>
            <button
              type="button"
              className="apps-action"
              onClick={() => setConfirmUninstall(false)}
            >
              {i18n.t("apps.cancel")}
            </button>
          </div>
        </div>
      ) : null}

      {app.install_progress ? (
        <div className="apps-progress">
          <div className="apps-progress-label">
            <span>{app.install_progress.phase}</span>
            {app.install_progress.percent !== null ? (
              <span>{app.install_progress.percent}%</span>
            ) : null}
          </div>
          <div className="apps-progress-track">
            <div
              className="apps-progress-fill"
              style={{ width: `${app.install_progress.percent ?? 8}%` }}
            />
          </div>
          {app.install_progress.message ? (
            <div className="apps-progress-message">{app.install_progress.message}</div>
          ) : null}
        </div>
      ) : null}

      {app.install_error ? (
        <div className="apps-error">
          <div>{app.install_error.message}</div>
          {app.install_error.tail ? (
            <pre>{app.install_error.tail}</pre>
          ) : null}
        </div>
      ) : app.runtime_error ? (
        <div className="apps-error">{app.runtime_error}</div>
      ) : null}

      {showTrust ? (
        <div className="apps-trust">
          <div className="apps-section-title">{i18n.t("apps.trustTitle")}</div>
          <p className="apps-hint">{i18n.t("apps.trustHint")}</p>
          <pre className="apps-pre">{app.trust_preview}</pre>
          <button
            type="button"
            className="apps-action"
            disabled={!!busy || !app.definition_hash}
            onClick={() =>
              void action(() =>
                app.definition_hash
                  ? trust(app.id, app.definition_hash)
                  : Promise.resolve(),
              )
            }
          >
            {i18n.t("apps.trust")}
          </button>
        </div>
      ) : null}

      {isBuiltin ? null : (
        <>
          <div className="apps-section-title">{i18n.t("apps.readme")}</div>
          {readme !== null ? (
            <div className="apps-readme">
              <AgentMarkdown text={readme} />
            </div>
          ) : readmeError ? (
            <p className="apps-hint">{app.summary || i18n.t("apps.readmeUnavailable")}</p>
          ) : (
            <p className="apps-hint">{i18n.t("apps.loading")}</p>
          )}
        </>
      )}

      {isBuiltin ? null : (
        <div className="apps-paths">
          <div>
            <span className="apps-path-label">{i18n.t("apps.installDir")}</span>
            <code>{app.install_dir}</code>
          </div>
          <div>
            <span className="apps-path-label">{i18n.t("apps.dataDir")}</span>
            <code>{app.data_dir}</code>
          </div>
        </div>
      )}

      {app.permissions.length > 0 ? (
        <div className="apps-permissions">
          <span className="apps-path-label">{i18n.t("apps.permissions")}</span>
          <span>{app.permissions.join(", ")}</span>
        </div>
      ) : null}

      {app.install_error && app.install_error.log_path ? (
        <div className="apps-log-path">
          {i18n.t("apps.log")}: <code>{app.install_error.log_path}</code>
        </div>
      ) : null}
    </div>
  );
}
