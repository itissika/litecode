import { useEffect, useRef } from "react";
import { CaretDown, CaretUp } from "@phosphor-icons/react";

import { useConnectionStore } from "../stores/connectionStore";
import {
  formatMemoryLabel,
  formatMemoryTitle,
  useTelemetryStore,
} from "../stores/telemetryStore";
import type { LogLine } from "../api/types";

function formatLogTime(tsMs: number): string {
  const d = new Date(tsMs);
  return d.toLocaleTimeString(undefined, { hour12: false });
}

function logLevelClass(level: string): string {
  switch (level.toUpperCase()) {
    case "ERROR":
      return "status-log-error";
    case "WARN":
      return "status-log-warn";
    case "DEBUG":
    case "TRACE":
      return "status-log-debug";
    default:
      return "status-log-info";
  }
}

function LogRow({ line }: { line: LogLine }) {
  return (
    <div className={`status-log-row ${logLevelClass(line.level)}`}>
      <span className="status-log-time">{formatLogTime(line.ts_ms)}</span>
      <span className="status-log-level">{line.level}</span>
      <span className="status-log-target" title={line.target}>
        {line.target}
      </span>
      <span className="status-log-message">{line.message}</span>
    </div>
  );
}

interface StatusBarProps {
  sessionMode?: "local" | "remote";
}

export function StatusBar({ sessionMode = "local" }: StatusBarProps) {
  const connection = useConnectionStore((s) => s.state);
  const memory = useTelemetryStore((s) => s.memory);
  const logsExpanded = useTelemetryStore((s) => s.logsExpanded);
  const logLines = useTelemetryStore((s) => s.logLines);
  const setExpanded = useTelemetryStore((s) => s.setExpanded);

  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !logsExpanded) return;

    const onScroll = () => {
      const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 32;
      stickToBottom.current = nearBottom;
    };
    el.addEventListener("scroll", onScroll);
    return () => el.removeEventListener("scroll", onScroll);
  }, [logsExpanded]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !logsExpanded || !stickToBottom.current) return;
    el.scrollTop = el.scrollHeight;
  }, [logLines, logsExpanded]);

  const rssLabel =
    connection !== "connected" ? "RSS —" : formatMemoryLabel(memory);
  const rssTitle =
    connection !== "connected"
      ? "Server resident memory"
      : formatMemoryTitle(memory);

  return (
    <div className="status-bar-root">
      {logsExpanded ? (
        <div ref={scrollRef} className="status-log-panel" aria-label="Server logs">
          {logLines.length === 0 ? (
            <p className="status-log-empty">Waiting for log events…</p>
          ) : (
            logLines.map((line, i) => (
              <LogRow key={`${line.ts_ms}-${i}`} line={line} />
            ))
          )}
        </div>
      ) : null}

      <div className="status-bar-strip">
        {sessionMode === "remote" ? (
          <span
            className="status-bar-rss"
            title="Connected to a remote litecode serve (no local sidecar)"
          >
            Remote
          </span>
        ) : null}
        <span className="status-bar-rss" title={rssTitle}>
          {rssLabel}
        </span>
        <button
          type="button"
          className="status-bar-toggle"
          onClick={() => setExpanded(!logsExpanded)}
          aria-expanded={logsExpanded}
          aria-label={logsExpanded ? "Collapse server logs" : "Expand server logs"}
        >
          <span>Logs</span>
          {logsExpanded ? (
            <CaretDown size={12} weight="bold" aria-hidden />
          ) : (
            <CaretUp size={12} weight="bold" aria-hidden />
          )}
        </button>
      </div>
    </div>
  );
}
