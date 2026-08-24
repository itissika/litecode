import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { catalogPollDelayMs, resetCatalogPollState, useSettingsStore } from "./settingsStore";
import { useToastStore } from "./toastStore";
import { registerSettingsFlush } from "../dockview/panels/settings/persist";
import type { EnginesDetail } from "../api/workspace";
import type { EngineWarmupState } from "../api/settings";

vi.mock("../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/workspace")>();
  return {
    ...actual,
    getEnginesDetail: vi.fn(),
  };
});

import { getEnginesDetail } from "../api/workspace";

const mockedGetEnginesDetail = vi.mocked(getEnginesDetail);

function enginesDetail(state: EngineWarmupState): EnginesDetail {
  return {
    retrieval: {
      desired: false,
      state: "stopped",
      usable: "stopped",
      error: null,
      model: {
        model_found: false,
        tokenizer_found: false,
        ready: false,
      },
      index: {
        status: "absent",
        exists: false,
        needs_rebuild: false,
        vectors_ready: false,
        indexed_files: 0,
        indexed_chunks: 0,
      },
      policy: {
        product_internal_dirs: [],
        exclude_globs: [],
        extensions: [],
        max_file_bytes: 0,
        binary_files: false,
        lockfiles: false,
        minified_files: false,
      },
    },
    lsp: {
      desired: true,
      state,
      usable: state === "warm" ? "ready" : "warming",
      configured_servers: [],
      probes: [],
    },
  };
}

beforeEach(() => {
  resetCatalogPollState();
  useSettingsStore.setState({
    engineStatuses: {},
  });
  useToastStore.setState({ toasts: [] });
  mockedGetEnginesDetail.mockReset();
});

afterEach(() => {
  resetCatalogPollState();
  vi.useRealTimers();
});

describe("catalogPollDelayMs (FE-03 backoff sequence)", () => {
  it("grows exponentially from the base and caps at the maximum", () => {
    const sequence = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(catalogPollDelayMs);
    expect(sequence).toEqual([
      500, 1000, 2000, 4000, 8000, 8000, 8000, 8000, 8000, 8000,
    ]);
    expect(sequence[0]).toBe(500);
    expect(sequence[4]).toBe(8000);
    expect(sequence[9]).toBe(8000);
  });
});

describe("ensureCatalogLoaded warming poll (FE-03)", () => {
  it("keeps polling with backoff while an engine warms, then settles", async () => {
    vi.useFakeTimers();
    mockedGetEnginesDetail
      .mockResolvedValueOnce(enginesDetail("warming"))
      .mockResolvedValueOnce(enginesDetail("warming"))
      .mockResolvedValueOnce(enginesDetail("warm"));

    void useSettingsStore.getState().ensureCatalogLoaded();
    await vi.advanceTimersByTimeAsync(500); // poll #2
    await vi.advanceTimersByTimeAsync(1000); // poll #3 → warm

    expect(mockedGetEnginesDetail).toHaveBeenCalledTimes(3);
    expect(useSettingsStore.getState().engineStatuses.lsp.state).toBe("warm");
  });

  it("keeps polling quietly after the cap while an engine stays warming — no error toast", async () => {
    vi.useFakeTimers();
    mockedGetEnginesDetail.mockResolvedValue(enginesDetail("warming"));

    void useSettingsStore.getState().ensureCatalogLoaded();
    // Poll delays: 500, 1000, 2000, 4000, then 8000… (cap).
    const delays = [500, 1000, 2000, 4000, 8000, 8000, 8000, 8000];
    for (const delay of delays) {
      await vi.advanceTimersByTimeAsync(delay);
    }

    expect(mockedGetEnginesDetail).toHaveBeenCalledTimes(9);
    expect(useToastStore.getState().toasts).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(8000);
    expect(mockedGetEnginesDetail).toHaveBeenCalledTimes(10);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("toasts a sustained catalog fetch error once, then retries without repeating the toast", async () => {
    vi.useFakeTimers();
    mockedGetEnginesDetail.mockRejectedValue(new Error("catalog fetch failed"));
    const shown: string[] = [];
    let toastCount = 0;
    const unsub = useToastStore.subscribe((s) => {
      if (s.toasts.length > toastCount) {
        shown.push(s.toasts[s.toasts.length - 1].message);
      }
      toastCount = s.toasts.length;
    });

    void useSettingsStore.getState().ensureCatalogLoaded();
    const delays = [500, 1000, 2000, 4000, 8000, 8000, 8000, 8000];
    for (const delay of delays) {
      await vi.advanceTimersByTimeAsync(delay);
    }

    expect(shown).toEqual(["catalog fetch failed"]);

    await vi.advanceTimersByTimeAsync(8000);
    expect(mockedGetEnginesDetail.mock.calls.length).toBeGreaterThan(9);
    expect(shown).toEqual(["catalog fetch failed"]);
    unsub();
  });

  it("retries a transient fetch error with backoff and does not swallow it silently", async () => {
    vi.useFakeTimers();
    mockedGetEnginesDetail
      .mockRejectedValueOnce(new Error("catalog fetch failed"))
      .mockResolvedValueOnce(enginesDetail("warm"));

    void useSettingsStore.getState().ensureCatalogLoaded();
    await vi.advanceTimersByTimeAsync(500); // error → retry poll #2

    expect(mockedGetEnginesDetail).toHaveBeenCalledTimes(2);
    expect(useSettingsStore.getState().engineStatuses.lsp.state).toBe("warm");
  });
});

describe("settings persist toasts", () => {
  it("does not success-toast settings/changed while the dialog is open", () => {
    useToastStore.setState({ toasts: [] });
    useSettingsStore.setState({ open: true, persistStatus: "saving" });
    useSettingsStore.getState().onRemoteSettingsChanged({
      revision: 99,
      summary: {
        revision: 99,
        provider_endpoint: null,
        model_count: 1,
        agent_count: 1,
        catalog_count: 1,
        log_level: "info",
        effective_next_turn: true,
        restart_required: false,
      },
    });
    expect(useToastStore.getState().toasts.map((t) => t.message)).not.toContain(
      "Settings changed — effective next turn",
    );
  });

  it("flushes registered persist before closing settings", async () => {
    const flush = vi.fn(async () => undefined);
    const unreg = registerSettingsFlush(flush);
    await useSettingsStore.getState().closeSettings();
    expect(flush).toHaveBeenCalledTimes(1);
    unreg();
  });
});
