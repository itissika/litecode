import { create } from "zustand";

import type { EngineStatus } from "../api/settings";
import {
  getEngines,
  type EnginesDetail,
  type LspInstanceStatusView,
} from "../api/workspace";
import { useToastStore } from "./toastStore";

const WARMUP_BASE_MS = 500;
const WARMUP_MAX_DELAY_MS = 8000;
const WARMUP_MAX_ATTEMPTS = 8;
let warmupAttempts = 0;
let catalogPollTimer: ReturnType<typeof setTimeout> | null = null;
let catalogFetchErrorToasted = false;

/** Exponential backoff delay (ms) for the `attempt`-th catalog poll (1-based). */
export function catalogPollDelayMs(attempt: number): number {
  const n = Math.max(0, attempt - 1);
  return Math.min(WARMUP_BASE_MS * 2 ** n, WARMUP_MAX_DELAY_MS);
}

function clearCatalogPollTimer(): void {
  if (catalogPollTimer !== null) {
    clearTimeout(catalogPollTimer);
    catalogPollTimer = null;
  }
}

/** Test hook: module poll state is not in the zustand store. */
export function resetCatalogPollState(): void {
  warmupAttempts = 0;
  catalogFetchErrorToasted = false;
  detailPolling = false;
  clearCatalogPollTimer();
}

function markCatalogSettled(): void {
  warmupAttempts = 0;
  catalogFetchErrorToasted = false;
  clearCatalogPollTimer();
}

/** When Engines page polls /engines/detail, cheap GET /engines must not also poll. */
let detailPolling = false;

export function setEngineDetailPolling(on: boolean): void {
  detailPolling = on;
  if (on) {
    clearCatalogPollTimer();
    return;
  }
  const warming = Object.values(useEngineStore.getState().engineStatuses).some(
    (engine) => engine?.state === "warming",
  );
  if (warming) scheduleCatalogPoll();
}

function scheduleCatalogPoll(error?: unknown): void {
  if (detailPolling) return;
  if (catalogPollTimer !== null) {
    return;
  }
  warmupAttempts += 1;
  if (warmupAttempts > WARMUP_MAX_ATTEMPTS) {
    warmupAttempts = WARMUP_MAX_ATTEMPTS;
    if (error instanceof Error && !catalogFetchErrorToasted) {
      catalogFetchErrorToasted = true;
      useToastStore.getState().showToast(error.message, "error");
    }
  }
  const delay = catalogPollDelayMs(warmupAttempts);
  catalogPollTimer = setTimeout(() => {
    catalogPollTimer = null;
    void useEngineStore.getState().ensureLoaded();
  }, delay);
}

export function enginesFromDetail(detail: EnginesDetail): Record<string, EngineStatus> {
  return {
    lsp: {
      desired: detail.lsp.desired,
      state: detail.lsp.state,
      error: detail.lsp.error,
    },
    code_search: {
      desired: detail.retrieval.desired,
      state: detail.retrieval.state,
      error: detail.retrieval.error,
    },
  };
}

interface EngineStore {
  engineStatuses: Record<string, EngineStatus>;
  lspServers: LspInstanceStatusView[];
  ensureLoaded: () => Promise<void>;
  applyFromDetail: (detail: EnginesDetail) => void;
}

export const useEngineStore = create<EngineStore>((set) => ({
  engineStatuses: {},
  lspServers: [],

  ensureLoaded: async () => {
    try {
      const snap = await getEngines();
      const engineStatuses = snap.engines ?? {};
      set({
        engineStatuses,
        lspServers: snap.lsp_servers ?? [],
      });
      const warming = Object.values(engineStatuses).some(
        (engine) => engine?.state === "warming",
      );
      if (warming) {
        scheduleCatalogPoll();
      } else {
        markCatalogSettled();
      }
    } catch (err) {
      scheduleCatalogPoll(err);
    }
  },

  applyFromDetail: (detail) => {
    const engineStatuses = enginesFromDetail(detail);
    set({
      engineStatuses,
      lspServers: detail.lsp.servers ?? [],
    });
    const warming = Object.values(engineStatuses).some(
      (engine) => engine?.state === "warming",
    );
    if (warming) {
      scheduleCatalogPoll();
    } else {
      markCatalogSettled();
    }
  },
}));
