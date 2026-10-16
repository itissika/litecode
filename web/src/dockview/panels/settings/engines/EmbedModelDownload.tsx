import { useCallback, useEffect, useRef, useState } from "react";
import { CloudArrowDown, WarningCircle } from "@phosphor-icons/react";

import type { ComponentProgressEvent } from "../../../../api/auth";
import { glassFill } from "../../../../components/composerCard";

/**
 * Slim installer: the embed model is not shipped in the installer. The
 * Electron host downloads it on demand (GitHub Release of this version,
 * sha256-verified) into %LOCALAPPDATA%\litecode\bundles. Only offered in a
 * local desktop session; browser dev / remote workspaces keep the plain
 * "not found" line.
 */

type EmbedDownloadState =
  | { phase: "idle" }
  | { phase: "running"; message: string; percent: number }
  | { phase: "error"; error: string }
  | { phase: "done" };

const COMPONENT = "embed-model" as const;

/** Strip Electron's `Error invoking remote method '…': Error: ` wrapper. */
export function cleanIpcError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  return raw.replace(/^Error invoking remote method '[^']+':\s*(Error:\s*)?/, "").trim() || "Download failed";
}

export function embedDownloadAvailable(): boolean {
  if (typeof window === "undefined") return false;
  const bridge = window.litecode;
  return Boolean(bridge?.ensureComponent) && bridge?.getSessionMode?.() !== "remote";
}

export function useEmbedModelDownload(onInstalled: () => Promise<void> | void) {
  const available = embedDownloadAvailable();
  const [state, setState] = useState<EmbedDownloadState>({ phase: "idle" });
  const onInstalledRef = useRef(onInstalled);
  onInstalledRef.current = onInstalled;
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!available) return;
    const bridge = window.litecode!;
    const stop = bridge.onComponentProgress?.((event: ComponentProgressEvent) => {
      if (event.id !== COMPONENT || !mounted.current) return;
      // Settle from events too: the download may have been started by another
      // view (Start button, Open Remote) whose ensure() this hook never sees.
      if (event.phase === "done") {
        setState({ phase: "done" });
        return;
      }
      if (event.phase === "error") {
        setState({ phase: "error", error: event.error || event.message || "Download failed" });
        return;
      }
      setState({ phase: "running", message: event.message, percent: event.percent });
    });
    // A download started elsewhere (e.g. before this view mounted) keeps running in the host.
    void bridge.getComponentStatus?.(COMPONENT).then((status) => {
      if (!mounted.current) return;
      if (status.downloading) {
        setState((s) => (s.phase === "idle" ? { phase: "running", message: "Downloading embedding model…", percent: 0 } : s));
      } else if (status.error) {
        setState((s) => (s.phase === "idle" ? { phase: "error", error: status.error! } : s));
      }
    }).catch(() => undefined);
    return () => stop?.();
  }, [available]);

  const start = useCallback(async () => {
    const bridge = window.litecode;
    if (!bridge?.ensureComponent) return;
    setState({ phase: "running", message: "Looking up embedding model…", percent: 0 });
    try {
      await bridge.ensureComponent(COMPONENT);
      if (!mounted.current) return;
      setState({ phase: "done" });
      await onInstalledRef.current();
    } catch (error) {
      if (mounted.current) setState({ phase: "error", error: cleanIpcError(error) });
    }
  }, []);

  return { available, state, start };
}

export function EmbedModelDownloadCard({
  state,
  onStart,
}: {
  state: EmbedDownloadState;
  onStart: () => void;
}) {
  const running = state.phase === "running";
  // Contract: percent is always present; 0 means not started or size unknown.
  const pct = running && state.percent > 0 ? state.percent : null;
  const subtitle =
    state.phase === "running"
      ? state.message
      : state.phase === "done"
        ? "Installed — starting retrieval…"
        : "Semantic retrieval needs a one-time download from the Litecode GitHub release (sha256-verified).";
  return (
    <div
      className={`embed-download-card rounded-md border border-(--_dk-line) p-3 ${glassFill}`}
      data-phase={state.phase}
      role="group"
      aria-label="Embedding model download"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2">
          <CloudArrowDown size={16} className="mt-0.5 shrink-0 text-(--_dk-accent-hover)" aria-hidden />
          <div className="min-w-0">
            <p className="text-xs font-medium text-(--_dk-text-primary)">Embedding model not installed</p>
            <p className="mt-0.5 text-dk-xs text-(--_dk-text-muted)" aria-live="polite">
              {subtitle}
            </p>
          </div>
        </div>
        {state.phase === "idle" || state.phase === "error" ? (
          <button type="button" className="btn btn-sm shrink-0" onClick={onStart}>
            {state.phase === "error" ? "Retry" : "Download"}
          </button>
        ) : null}
      </div>
      {running ? (
        <div className="mt-2.5 flex items-center gap-2">
          <div className="engine-index-track min-w-0 flex-1">
            <div
              className={`engine-index-fill ${pct === null ? "is-pulse" : ""}`}
              data-tone="busy"
              style={{ width: pct === null ? "35%" : `${pct}%` }}
            />
          </div>
          <span className="w-9 shrink-0 text-right font-mono text-dk-2xs text-(--_dk-text-muted)">
            {pct === null ? "…" : `${pct}%`}
          </span>
        </div>
      ) : null}
      {state.phase === "error" ? (
        <p className="mt-2 flex items-start gap-1.5 text-dk-xs text-(--_dk-red-500)">
          <WarningCircle size={14} weight="fill" className="mt-px shrink-0" aria-hidden />
          <span className="min-w-0 whitespace-pre-wrap break-words">{state.error}</span>
        </p>
      ) : null}
    </div>
  );
}
