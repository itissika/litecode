import { create } from "zustand";

import { useConnectionStore, attachSiblingStores } from "./connectionStore";
import type { ConnectionState, LogLine, ServerStats } from "../api/types";

const MAX_LOG_LINES = 500;

export function formatRssMb(rssKb: number): string {
  const mb = rssKb / 1024;
  if (mb >= 100) {
    return `${Math.round(mb)}M`;
  }
  return `${mb.toFixed(1)}M`;
}

export interface MemoryBreakdown {
  totalKb: number | null;
  coreKb: number | null;
  embedKb: number | null;
  lspKb: number | null;
}

export function memoryFromStats(stats: ServerStats): MemoryBreakdown {
  return {
    totalKb: stats.rss_kb,
    coreKb: stats.core_rss_kb ?? stats.rss_kb,
    embedKb: stats.embed_rss_kb ?? 0,
    lspKb: stats.lsp_rss_kb ?? 0,
  };
}

export function formatMemoryLabel(memory: MemoryBreakdown): string {
  if (memory.totalKb == null) {
    return "RSS —";
  }
  const parts = [`${formatRssMb(memory.totalKb)} total`];
  if (memory.coreKb != null) {
    parts.push(`core ${formatRssMb(memory.coreKb)}`);
  }
  const embed = memory.embedKb ?? 0;
  const lsp = memory.lspKb ?? 0;
  if (embed > 0) {
    parts.push(`embed ${formatRssMb(embed)}`);
  }
  if (lsp > 0) {
    parts.push(`lsp ${formatRssMb(lsp)}`);
  }
  return parts.join(" · ");
}

export function formatMemoryTitle(memory: MemoryBreakdown): string {
  if (memory.totalKb == null) {
    return "Server resident memory";
  }
  const core = memory.coreKb ?? 0;
  const embed = memory.embedKb ?? 0;
  const lsp = memory.lspKb ?? 0;
  return [
    `Total: ${formatRssMb(memory.totalKb)}`,
    `Core (serve): ${formatRssMb(core)}`,
    `Embed (code_search worker): ${formatRssMb(embed)}`,
    `LSP: ${formatRssMb(lsp)}`,
  ].join("\n");
}

function appendLogLine(lines: LogLine[], line: LogLine): LogLine[] {
  const next = [...lines, line];
  if (next.length <= MAX_LOG_LINES) {
    return next;
  }
  return next.slice(next.length - MAX_LOG_LINES);
}

/** @internal test helper */
export function appendLogLineForTest(
  lines: LogLine[],
  batch: LogLine[],
): LogLine[] {
  return batch.reduce(appendLogLine, lines);
}

function sendSubscribe(subscribe: boolean): void {
  try {
    const conn = useConnectionStore.getState();
    if (conn.state !== "connected") return;
    conn.sendRpc(
      subscribe ? "subscribe_logs" : "unsubscribe_logs",
    ).catch(() => {});
  } catch {
    // WS not ready yet; expand will retry on next connect if needed
  }
}

interface TelemetryStore {
  memory: MemoryBreakdown;
  logsExpanded: boolean;
  logLines: LogLine[];
  onServerStats: (stats: ServerStats) => void;
  onLogLine: (line: LogLine) => void;
  setExpanded: (open: boolean) => void;
  clearLogs: () => void;
  reset: () => void;
  onConnectionChange: (connection: ConnectionState) => void;
}

const emptyMemory: MemoryBreakdown = {
  totalKb: null,
  coreKb: null,
  embedKb: null,
  lspKb: null,
};

export const useTelemetryStore = create<TelemetryStore>((set, get) => ({
  memory: emptyMemory,
  logsExpanded: false,
  logLines: [],

  onServerStats: (stats) => {
    set({ memory: memoryFromStats(stats) });
  },

  onLogLine: (line) => {
    set((s) => ({ logLines: appendLogLine(s.logLines, line) }));
  },

  setExpanded: (open) => {
    const wasExpanded = get().logsExpanded;
    if (open === wasExpanded) return;

    if (open) {
      set({ logsExpanded: true });
      sendSubscribe(true);
      return;
    }

    sendSubscribe(false);
    set({ logsExpanded: false, logLines: [] });
  },

  clearLogs: () => set({ logLines: [] }),

  reset: () => {
    if (get().logsExpanded) {
      sendSubscribe(false);
    }
    set({ memory: emptyMemory, logsExpanded: false, logLines: [] });
  },

  onConnectionChange: (connection) => {
    if (connection === "disconnected") {
      get().reset();
      return;
    }
    if (connection === "connected" && get().logsExpanded) {
      sendSubscribe(true);
    }
  },
}));

attachSiblingStores({ telemetry: useTelemetryStore });
