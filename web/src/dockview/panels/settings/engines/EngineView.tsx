import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import {
  ArrowsClockwise,
  CheckCircle,
  Info,
  Play,
  Stop,
  WarningCircle,
  XCircle,
} from "@phosphor-icons/react";

import {
  getInstallStatus,
  installServer,
  probeLspServers,
  refreshRetrieval,
  type EnginesDetail,
  type IndexStatus,
  type IndexingProgress,
  type LspServerProbe,
  type RetrievalEngineDetail,
} from "../../../../api/workspace";
import type { WorkspaceEnginesDoc } from "../../../../api/settings";
import { useSettingsStore } from "../../../../stores/settingsStore";
import { useToastStore } from "../../../../stores/toastStore";
import {
  SETTINGS_PERSIST_ERROR_CHANNEL,
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "../persist";

const DEFAULT_ENGINES: WorkspaceEnginesDoc = {
  version: 1,
  lsp: { desired: false, servers: [] },
  retrieval: { desired: false },
};

function snapshotEngines(): WorkspaceEnginesDoc {
  return useSettingsStore.getState().engines ?? DEFAULT_ENGINES;
}

/* ------------------------------------------------------------------ */
/* Retrieval section                                                   */
/* ------------------------------------------------------------------ */

type EngineUsable = RetrievalEngineDetail["usable"];
type IndexTone = "ready" | "stale" | "rebuild" | "busy" | "idle";

function engineTag(usable: EngineUsable): {
  label: string;
  tone: "ok" | "warn" | "neutral" | "danger";
} {
  switch (usable) {
    case "ready":
      return { label: "ready", tone: "ok" };
    case "warming":
      return { label: "warming", tone: "warn" };
    case "stopped":
      return { label: "stopped", tone: "neutral" };
    default:
      return { label: "unavailable", tone: "danger" };
  }
}

/** Play/stop follows persisted desired, not runtime usable. Warming does not disable. */
function intentControl(desired: boolean): {
  icon: "play" | "stop";
  action: "start" | "stop";
  label: string;
} {
  if (desired) {
    return { icon: "stop", action: "stop", label: "Stop engine" };
  }
  return { icon: "play", action: "start", label: "Start engine" };
}

function resolveIndexStatus(index: RetrievalEngineDetail["index"]): IndexStatus {
  return index.status ?? (index.needs_rebuild ? "needs_rebuild" : index.exists ? "ready" : "absent");
}

function indexTone(status: IndexStatus, engineUsable: EngineUsable): IndexTone {
  if (engineUsable === "stopped" || engineUsable === "unavailable") return "idle";
  switch (status) {
    case "ready":
      return "ready";
    case "stale":
      return "stale";
    case "needs_rebuild":
    case "failed":
      return "rebuild";
    case "building":
    case "refreshing":
      return "busy";
    case "absent":
    default:
      return "idle";
  }
}

function barWidth(status: IndexStatus, progress?: IndexingProgress | null): { width: string; pulse: boolean } {
  if (status === "building" || status === "refreshing") {
    if (progress && progress.files_total > 0) {
      const pct = Math.min(100, Math.round((progress.files_done * 100) / progress.files_total));
      return { width: `${pct}%`, pulse: true };
    }
    return { width: "45%", pulse: true };
  }
  if (status === "absent") return { width: "0%", pulse: false };
  return { width: "100%", pulse: false };
}

function modelDisplay(model: RetrievalEngineDetail["model"]): { name: string; path: string } {
  const path = (model.model_dir || model.model_path || "").replace(/\\/g, "/");
  const parts = path.split("/").filter(Boolean);
  let name = parts[parts.length - 1] || "model";
  if (name.endsWith(".onnx") || name === "artifacts") {
    name = parts[parts.length - 2] || name;
    if (name === "artifacts") name = parts[parts.length - 3] || name;
  }
  return { name, path: path || "(unknown)" };
}

function formatRelative(iso?: string | null): string | null {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const mins = Math.max(0, Math.floor((Date.now() - then) / 60_000));
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function IconSquareButton({
  label,
  disabled,
  onClick,
  size = "md",
  children,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  size?: "xs" | "sm" | "md" | "lg";
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={`btn btn-icon${size !== "md" ? ` btn-${size}` : ""}`}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function RetrievalSection({
  detail,
  onChanged,
}: {
  detail: RetrievalEngineDetail;
  onChanged: () => void;
}) {
  const engines = useSettingsStore((s) => s.engines) ?? DEFAULT_ENGINES;
  const saveEngines = useSettingsStore((s) => s.saveEngines);
  const [busy, setBusy] = useState(false);
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await action();
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  const tag = engineTag(detail.usable);
  const desired = engines.retrieval.desired;
  const control = intentControl(desired);
  const index = detail.index;
  const status = resolveIndexStatus(index);
  const tone = indexTone(status, detail.usable);
  const { width, pulse } = barWidth(status, index.progress);
  const model = modelDisplay(detail.model);
  const relative = formatRelative(index.created_at);
  const refreshDisabled =
    busy ||
    tone === "busy" ||
    detail.usable === "warming" ||
    (detail.usable === "unavailable" && !detail.model.ready);
  const pending = index.pending_updates ?? 0;
  const work = index.work;
  const workTip =
    work?.kind === "rebuild"
      ? "needs rebuild"
      : work?.kind === "update"
        ? `${work.dirty} to update`
        : pending > 0
          ? `${pending} pending`
          : "";
  const stats =
    index.indexed_files > 0 || index.indexed_chunks > 0
      ? `${index.indexed_files} files · ${index.indexed_chunks} chunks`
      : status === "building" || status === "refreshing"
        ? index.progress
          ? `${index.progress.files_done}/${index.progress.files_total || "?"} files · ${index.progress.chunks_done} chunks`
          : "indexing…"
        : "no index stats";
  const detailTip = workTip ? `${stats} · ${workTip}` : stats;
  const skipTip = (() => {
    const dirs = detail.policy.product_internal_dirs ?? [];
    const globs = detail.policy.exclude_globs ?? [];
    const parts: string[] = [];
    if (dirs.length > 0) parts.push(`Internal dirs: ${dirs.join(", ")}`);
    if (globs.length > 0) parts.push(`Exclude: ${globs.join(", ")}`);
    return parts.length > 0 ? parts.join(" · ") : "No skip paths";
  })();

  const onEngineClick = () => {
    const nextDesired = control.action === "start";
    void run(() =>
      saveEngines({
        ...snapshotEngines(),
        retrieval: { desired: nextDesired },
      }),
    );
  };

  const ControlIcon = control.icon === "play" ? Play : Stop;

  return (
    <section className="space-y-3">
      <div className="settings-op-divider flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-medium text-(--_dk-text-primary)">Semantic retrieval</h3>
          <span className={`tag tag-${tag.tone} tag-md`}>{tag.label}</span>
        </div>
        <IconSquareButton
          label={control.label}
          disabled={busy}
          onClick={onEngineClick}
        >
          <ControlIcon size={14} weight="fill" />
        </IconSquareButton>
      </div>

      <div className="space-y-3 pl-4">
        {detail.error && (
          <p className="truncate text-dk-xs text-(--_dk-red-500)" title={detail.error}>{detail.error}</p>
        )}

        <div className="flex min-w-0 items-center gap-2 text-xs text-(--_dk-text-secondary)">
          {detail.model.ready ? (
            <CheckCircle size={14} weight="fill" className="shrink-0 text-(--_dk-emerald-500)" aria-hidden />
          ) : (
            <XCircle size={14} weight="fill" className="shrink-0 text-(--_dk-red-500)" aria-hidden />
          )}
          <span className="min-w-0 break-all font-mono text-dk-xs">
            {detail.model.ready
              ? model.name
              : `not found: ${model.name} under ${model.path}`}
          </span>
        </div>

        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <div
              className="engine-index-track min-w-0 flex-1"
              title={detailTip}
              data-tone={tone}
            >
              <div
                className={`engine-index-fill ${pulse ? "is-pulse" : ""}`}
                data-tone={tone}
                style={{ width }}
              />
            </div>
            <IconSquareButton
              label="Refresh index"
              size="xs"
              disabled={refreshDisabled}
              onClick={() => void run(refreshRetrieval)}
            >
              <ArrowsClockwise size={14} weight="bold" className={tone === "busy" ? "animate-spin" : undefined} />
            </IconSquareButton>
            <span className="engine-icon-static" title={skipTip} aria-label={skipTip}>
              <Info size={14} />
            </span>
          </div>
          {relative && (
            <p className="text-dk-xs text-(--_dk-text-disabled)">Updated {relative}</p>
          )}
        </div>
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* LSP section                                                         */
/* ------------------------------------------------------------------ */

function ActionButton({ children, onClick, disabled = false }: {
  children: ReactNode;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
}) {
  return (
    <button type="button" onClick={onClick} disabled={disabled} className="btn btn-sm">
      {children}
    </button>
  );
}

function InstallProgress({ progress }: { progress?: { downloaded_bytes: number; total_bytes?: number | null } | null }) {
  if (!progress) return <span className="text-xs text-(--_dk-text-muted)">Installing…</span>;
  const percent = progress.total_bytes ? Math.min(100, Math.round(progress.downloaded_bytes * 100 / progress.total_bytes)) : null;
  return (
    <div className="min-w-32">
      <div className="h-1.5 overflow-hidden rounded bg-(--_dk-line)">
        <div className={`h-full bg-(--_dk-accent-hover) ${percent === null ? "w-1/2 animate-pulse" : ""}`} style={percent === null ? undefined : { width: `${percent}%` }} />
      </div>
      <div className="mt-1 text-dk-2xs text-(--_dk-text-muted)">
        {percent === null ? `${Math.round(progress.downloaded_bytes / 1024 / 1024)} MB` : `${percent}%`}
      </div>
    </div>
  );
}

function LspServerCard({
  probe,
  checked,
  installing,
  onToggle,
  onInstall,
}: {
  probe: LspServerProbe;
  checked: boolean;
  installing?: { taskId: string; progress?: { downloaded_bytes: number; total_bytes?: number | null } | null };
  onToggle: () => void;
  onInstall: () => void;
}) {
  const ready = probe.status === "available";
  const summary = [
    probe.installed_version ? `v${probe.installed_version}` : null,
    ...probe.sources,
  ].filter(Boolean).join(" · ");
  return (
    <div className="tool-binding-card flex flex-col overflow-hidden" data-enabled={checked ? "true" : "false"}>
      <button
        type="button"
        aria-pressed={checked}
        disabled={!ready && !checked}
        onClick={onToggle}
        aria-label={`${probe.id} server, ${checked ? "enabled" : "disabled"}. Click the card to toggle.`}
        className="tool-binding-toggle"
      />
      <div className="tool-binding-content flex flex-col">
        <div className="flex min-h-[60px] w-full items-start justify-between gap-2 p-3">
          <div className="min-w-0">
            <p className="tool-binding-title truncate font-mono text-sm text-(--_dk-text-primary)">{probe.id}</p>
            <div className="mt-1 flex h-4 items-center gap-1.5">
              <span className="truncate text-dk-xs text-(--_dk-text-muted)">{summary}</span>
              {probe.error || (probe.managed_path && !ready) ? (
                <span
                  className="tool-binding-action shrink-0 cursor-help text-(--_dk-red-500)"
                  aria-label="error details"
                  title={[
                    probe.error,
                    !ready && probe.managed_path ? `Expected: ${probe.managed_path}` : null,
                  ]
                    .filter(Boolean)
                    .join("\n")}
                >
                  <WarningCircle size={14} weight="fill" />
                </span>
              ) : null}
            </div>
          </div>
          <span className={`tag ${checked ? "tag-ok" : "tag-neutral"} tag-sm tag-outline`}>
            {checked ? "On" : "Off"}
          </span>
        </div>
        <div className="tool-binding-foot flex shrink-0 items-center justify-between gap-2 px-3 py-2.5">
          <span className={ready ? "text-(--_dk-emerald-500)" : "text-(--_dk-amber-500)"}>
            {installing ? <InstallProgress progress={installing.progress} /> : probe.status}
          </span>
          {ready ? (
            <span className="shrink-0 text-(--_dk-emerald-500)" title="ready" aria-label="ready">
              <CheckCircle size={14} weight="fill" />
            </span>
          ) : (
            <div className="tool-binding-action flex items-center gap-1.5">
              <span className="shrink-0 text-(--_dk-red-500)" title="not installed" aria-label="not installed">
                <XCircle size={14} weight="fill" />
              </span>
              {probe.official_url && (
                <a
                  className="text-dk-xs text-(--_dk-accent-hover) underline"
                  href={probe.official_url}
                  target="_blank"
                  rel="noreferrer"
                  onClick={(event) => event.stopPropagation()}
                >
                  Guide
                </a>
              )}
              <ActionButton
                onClick={(event) => {
                  event.stopPropagation();
                  void onInstall();
                }}
                disabled={Boolean(installing)}
              >
                Install
              </ActionButton>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function LspSection({ detail, refresh }: { detail: EnginesDetail["lsp"]; refresh: () => void }) {
  const engines = useSettingsStore((s) => s.engines) ?? DEFAULT_ENGINES;
  const saveEngines = useSettingsStore((s) => s.saveEngines);
  const { persistStatus, setPersistStatus } = useDocPersist("engines");
  const [probes, setProbes] = useState<LspServerProbe[]>(detail.probes);
  const [selected, setSelected] = useState<Set<string>>(new Set(engines.lsp.servers));
  const [installing, setInstalling] = useState<Record<string, { taskId: string; progress?: { downloaded_bytes: number; total_bytes?: number | null } | null }>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const selectedIds = useMemo(() => [...selected].sort(), [selected]);

  const load = useCallback(async () => {
    const next = await probeLspServers();
    if (mountedRef.current) setProbes(next);
  }, []);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    setProbes(detail.probes);
  }, [detail]);
  useEffect(() => {
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    setSelected(new Set(engines.lsp.servers));
  }, [engines.lsp.servers, persistStatus]);

  useSettingsPersist(selectedIds, {
    debounceMs: 0,
    setStatus: setPersistStatus,
    serialize: (ids) => ({ ok: ids }),
    commit: async (ids) => {
      try {
        const current = snapshotEngines();
        await saveEngines({
          ...current,
          lsp: {
            servers: ids,
            desired: ids.length === 0 ? false : current.lsp.desired,
          },
        });
        refresh();
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        useToastStore.getState().showToast(
          message === "turn_in_progress"
            ? "Cannot change engines while an agent turn is in progress"
            : message,
          "error",
          5000,
          SETTINGS_PERSIST_ERROR_CHANNEL,
        );
        throw e;
      }
    },
    revert: () => setSelected(new Set(snapshotEngines().lsp.servers)),
  });

  const install = async (id: string) => {
    if (!mountedRef.current) return;
    try {
      const task = await installServer(id);
      if (!mountedRef.current) return;
      setInstalling((current) => ({ ...current, [id]: { taskId: task.task_id, progress: task.progress } }));
      let current = task;
      while (current.status === "installing") {
        await new Promise((resolve) => setTimeout(resolve, 800));
        if (!mountedRef.current) return;
        current = await getInstallStatus(task.task_id);
        if (!mountedRef.current) return;
        setInstalling((items) => ({ ...items, [id]: { taskId: task.task_id, progress: current.progress } }));
      }
      if (current.status === "failed") {
        setError(`${id}: ${current.error ?? "installation failed"}`);
      } else {
        setError(null);
      }
      await load();
    } catch (caught) {
      if (mountedRef.current) {
        setError(`${id}: ${caught instanceof Error ? caught.message : String(caught)}`);
      }
    } finally {
      if (mountedRef.current) {
        setInstalling((items) => {
          const next = { ...items };
          delete next[id];
          return next;
        });
      }
    }
  };

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await action();
      refresh();
    } finally {
      setBusy(false);
    }
  };

  // Engine ops: a single transport-style control (start ⇄ stop) driven by the
  // usable state, mirroring the retrieval section. The enabled set (card On/Off)
  // is the single source of truth and is persisted silently on every toggle.
  // Start requires ≥1 installed AND enabled language server.
  const hasRunnableServer = probes.some(
    (probe) => selected.has(probe.id) && probe.status === "available",
  );
  const desired = engines.lsp.desired;
  const control = intentControl(desired);
  const ControlIcon = control.icon === "play" ? Play : Stop;
  const controlDisabled = busy || (control.action === "start" && !hasRunnableServer);
  const onEngineClick = () => {
    const current = snapshotEngines();
    if (control.action === "start") {
      if (current.lsp.servers.length === 0) return;
      void run(() =>
        saveEngines({
          ...current,
          lsp: { ...current.lsp, desired: true },
        }),
      );
    } else {
      void run(() =>
        saveEngines({
          ...current,
          lsp: { ...current.lsp, desired: false },
        }),
      );
    }
  };

  // Toggle a card. Gate: a not-installed (×) server cannot be enabled; it can
  // only be turned off if already on. Persisted via useSettingsPersist.
  const toggle = (probe: LspServerProbe) => {
    const ready = probe.status === "available";
    const checked = selected.has(probe.id);
    if (!ready && !checked) return;
    const next = new Set(selected);
    next.has(probe.id) ? next.delete(probe.id) : next.add(probe.id);
    setSelected(next);
  };

  const tag = engineTag(detail.usable);

  return (
    <section className="space-y-3">
      {(error || detail.error) && (
        <p className="rounded border border-(--_dk-red-500) px-3 py-2 text-dk-xs text-(--_dk-red-500)">
          {error ?? detail.error}
        </p>
      )}
      <div className="settings-op-divider flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-medium text-(--_dk-text-primary)">Language servers</h3>
          <span className={`tag tag-${tag.tone} tag-md`}>{tag.label}</span>
        </div>
        <IconSquareButton
          label={control.label}
          disabled={controlDisabled}
          onClick={onEngineClick}
        >
          <ControlIcon size={14} weight="fill" />
        </IconSquareButton>
      </div>

      <div className="lsp-grid">
        {probes.map((probe) => (
          <LspServerCard
            key={probe.id}
            probe={probe}
            checked={selected.has(probe.id)}
            installing={installing[probe.id]}
            onToggle={() => toggle(probe)}
            onInstall={() => void install(probe.id)}
          />
        ))}
      </div>

      {(detail.servers?.length ?? 0) > 0 && (
        <div className="space-y-1 text-dk-xs text-(--_dk-text-secondary)">
          <p className="font-medium text-(--_dk-text-primary)">Running instances</p>
          <ul className="space-y-1">
            {detail.servers!.map((s) => (
              <li key={`${s.command}:${s.project_root}`}>
                {s.command} — {s.state}
                {s.index_settled ? "" : " (indexing)"}
                {s.last_error ? `: ${s.last_error}` : ""}
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Combined engine view                                                */
/* ------------------------------------------------------------------ */

export function EngineView({
  detail,
  onChanged,
}: {
  detail: EnginesDetail;
  onChanged: () => void;
}) {
  return (
    <div className="space-y-6">
      <RetrievalSection detail={detail.retrieval} onChanged={onChanged} />
      <LspSection detail={detail.lsp} refresh={onChanged} />
    </div>
  );
}
