import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { catalogPollDelayMs, resetCatalogPollState, setEngineDetailPolling, useEngineStore } from "./engineStore";
import { useToastStore } from "./toastStore";
import type { EnginesDetail, EnginesSnapshot } from "../api/workspace";
import type { EngineWarmupState } from "../api/settings";

vi.mock("../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/workspace")>();
  return {
    ...actual,
    getEngines: vi.fn(),
    getEnginesDetail: vi.fn(),
  };
});

import { getEngines, getEnginesDetail } from "../api/workspace";

const mockedGetEngines = vi.mocked(getEngines);
const mockedGetEnginesDetail = vi.mocked(getEnginesDetail);

function enginesSnap(state: EngineWarmupState): EnginesSnapshot {
  return {
    engines: {
      lsp: { desired: true, state },
      code_search: { desired: false, state: "stopped" },
    },
    lsp_servers: [],
  };
}

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
      servers: [],
    },
  };
}

beforeEach(() => {
  resetCatalogPollState();
  useEngineStore.setState({
    engineStatuses: {},
    lspServers: [],
  });
  useToastStore.setState({ toasts: [] });
  mockedGetEngines.mockReset();
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
  });
});

describe("ensureLoaded uses cheap GET /engines", () => {
  it("keeps polling with backoff while an engine warms, then settles", async () => {
    vi.useFakeTimers();
    mockedGetEngines
      .mockResolvedValueOnce(enginesSnap("warming"))
      .mockResolvedValueOnce(enginesSnap("warming"))
      .mockResolvedValueOnce(enginesSnap("warm"));

    void useEngineStore.getState().ensureLoaded();
    await vi.advanceTimersByTimeAsync(500);
    await vi.advanceTimersByTimeAsync(1000);

    expect(mockedGetEngines).toHaveBeenCalledTimes(3);
    expect(mockedGetEnginesDetail).not.toHaveBeenCalled();
    expect(useEngineStore.getState().engineStatuses.lsp.state).toBe("warm");
  });

  it("keeps polling quietly after the cap while an engine stays warming — no error toast", async () => {
    vi.useFakeTimers();
    mockedGetEngines.mockResolvedValue(enginesSnap("warming"));

    void useEngineStore.getState().ensureLoaded();
    const delays = [500, 1000, 2000, 4000, 8000, 8000, 8000, 8000];
    for (const delay of delays) {
      await vi.advanceTimersByTimeAsync(delay);
    }

    expect(mockedGetEngines).toHaveBeenCalledTimes(9);
    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(mockedGetEnginesDetail).not.toHaveBeenCalled();
  });

  it("toasts a sustained catalog fetch error once, then retries without repeating the toast", async () => {
    vi.useFakeTimers();
    mockedGetEngines.mockRejectedValue(new Error("catalog fetch failed"));
    const shown: string[] = [];
    let toastCount = 0;
    const unsub = useToastStore.subscribe((s) => {
      if (s.toasts.length > toastCount) {
        shown.push(s.toasts[s.toasts.length - 1].message);
      }
      toastCount = s.toasts.length;
    });

    void useEngineStore.getState().ensureLoaded();
    const delays = [500, 1000, 2000, 4000, 8000, 8000, 8000, 8000];
    for (const delay of delays) {
      await vi.advanceTimersByTimeAsync(delay);
    }

    expect(shown).toEqual(["catalog fetch failed"]);
    unsub();
  });

  it("applies detail without calling GET /engines/detail itself", () => {
    useEngineStore.getState().applyFromDetail(enginesDetail("warm"));
    expect(mockedGetEnginesDetail).not.toHaveBeenCalled();
    expect(useEngineStore.getState().engineStatuses.lsp.state).toBe("warm");
  });

  it("does not cheap-poll while the Engines page is detail-polling", async () => {
    vi.useFakeTimers();
    setEngineDetailPolling(true);
    mockedGetEngines.mockResolvedValue(enginesSnap("warming"));
    void useEngineStore.getState().ensureLoaded();
    await vi.advanceTimersByTimeAsync(8000);
    expect(mockedGetEngines).toHaveBeenCalledTimes(1);
    setEngineDetailPolling(false);
  });
});
