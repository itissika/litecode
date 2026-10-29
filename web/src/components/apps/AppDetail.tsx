import { useEffect, useState } from "react";

import {
  ArrowSquareOut,
  DownloadSimple,
  Play,
  Prohibit,
  Trash,
  X,
} from "@phosphor-icons/react";

import type { AppView } from "../../api/apps";
import { builtinAppName, openAppPanel, openBuiltinAppView } from "../../lib/appPanel";
import { appDescription } from "../../lib/appText";
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
  const install = useAppsStore((state) => state.install);
  const start = useAppsStore((state) => state.start);
  const stop = useAppsStore((state) => state.stop);
  const uninstall = useAppsStore((state) => state.uninstall);
  const trust = useAppsStore((state) => state.trust);
  const showToast = useToastStore((state) => state.showToast);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const isBuiltin = app.source === "builtin";
  const managed = app.install;
  const isInstalled = managed?.installed ?? true;
  const progress = managed?.progress ?? null;
  const installError = managed?.error ?? null;
  const docsText = app.docs
    ? i18n.language.toLowerCase().startsWith("zh")
      ? app.docs.zh
      : app.docs.en
    : null;

  useEffect(() => {
    setConfirmUninstall(false);
  }, [app.id]);

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

  const showTrust = !app.trusted && !!app.trust_preview;

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

      {app.repo || app.docs_url ? (
        <div className="apps-links">
          {app.repo ? (
            <a className="apps-link" href={app.repo} target="_blank" rel="noreferrer">
              <ArrowSquareOut size={12} />
              {i18n.t("apps.repo")}
            </a>
          ) : null}
          {app.docs_url ? (
            <a className="apps-link" href={app.docs_url} target="_blank" rel="noreferrer">
              <ArrowSquareOut size={12} />
              {i18n.t("apps.docs")}
            </a>
          ) : null}
        </div>
      ) : null}

      <div className="apps-detail-space">
        {app.builtin_view ? (
          <span>{i18n.t("apps.builtinHint")}</span>
        ) : isBuiltin ? (
          <span>{i18n.t("apps.managedHint")}</span>
        ) : (
          <span>{i18n.t("apps.externalHint")}</span>
        )}
        {app.single_instance ? <span>{i18n.t("apps.singleInstance")}</span> : null}
      </div>

      <div className="apps-detail-actions">
        {app.builtin_view ? (
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
        ) : isInstalled ? (
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
            disabled={!!busy || !managed?.can_install}
            onClick={() => void action(() => install(app.id))}
          >
            <DownloadSimple size={14} />
            {installError ? i18n.t("apps.retry") : i18n.t("apps.install")}
          </button>
        )}
        {isBuiltin && isInstalled && managed?.can_uninstall ? (
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
      </div>

      {confirmUninstall ? (
        <div className="apps-confirm">
          <span>{i18n.t("apps.uninstallConfirm", { name: builtinAppName(app) })}</span>
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

      {progress ? (
        <div className="apps-progress">
          <div className="apps-progress-label">
            <span>{progress.phase}</span>
            {progress.percent !== null ? <span>{progress.percent}%</span> : null}
          </div>
          <div className="apps-progress-track">
            <div
              className="apps-progress-fill"
              style={{ width: `${progress.percent ?? 8}%` }}
            />
          </div>
          {progress.message ? (
            <div className="apps-progress-message">{progress.message}</div>
          ) : null}
        </div>
      ) : null}

      {installError ? (
        <div className="apps-error">
          <div>{installError.message}</div>
          {installError.tail ? <pre>{installError.tail}</pre> : null}
        </div>
      ) : app.runtime_error ? (
        <div className="apps-error">{app.runtime_error}</div>
      ) : null}

      {docsText ? (
        <div className="apps-docs">
          <div className="apps-section-title">{i18n.t("apps.builtinDocs")}</div>
          <AgentMarkdown text={docsText} />
        </div>
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

      {app.permissions.length > 0 ? (
        <div className="apps-permissions">
          <span className="apps-path-label">{i18n.t("apps.permissions")}</span>
          <span>{app.permissions.join(", ")}</span>
        </div>
      ) : null}
    </div>
  );
}
