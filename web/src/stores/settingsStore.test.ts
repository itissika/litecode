import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { catalogPollDelayMs, useSettingsStore } from "./settingsStore";
import { useToastStore } from "./toastStore";

// Mock the settings API module so the catalog poll is deterministic.
vi.mock("../api/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/settings")>();
  return {
    ...actual,
    getToolCatalog: vi.fn(),
  };
});

import { getToolCatalog } from "../api/settings";
import type { EngineStatus, ToolCatalogEntry } from "../api/settings";

const mockedGetToolCatalog = vi.mocked(getToolCatalog);

function warmingEngines(state: EngineStatus["state"]): {
  tool_catalog: Record<string, ToolCatalogEntry>;
  engines: Record<string, EngineStatus>;
} {
  return {
    tool_catalog: {},
    engines: {
      lsp: { state },
    },
  };
}

beforeEach(() => {
  useSettingsStore.setState({
    toolCatalog: {},
    engineStatuses: {},
  });
  useToastStore.setState({ toasts: [] });
  mockedGetToolCatalog.mockReset();
});

afterEach(() => {
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
    mockedGetToolCatalog
      .mockResolvedValueOnce(warmingEngines("warming"))
      .mockResolvedValueOnce(warmingEngines("warming"))
      .mockResolvedValueOnce(warmingEngines("warm"));

    void useSettingsStore.getState().ensureCatalogLoaded();
    await vi.advanceTimersByTimeAsync(500); // poll #2
    await vi.advanceTimersByTimeAsync(1000); // poll #3 → warm

    expect(mockedGetToolCatalog).toHaveBeenCalledTimes(3);
    expect(useSettingsStore.getState().engineStatuses.lsp.state).toBe("warm");
  });

  it("surfaces an explicit toast after the poll cap instead of recursing forever", async () => {
    vi.useFakeTimers();
    mockedGetToolCatalog.mockResolvedValue(warmingEngines("warming"));

    void useSettingsStore.getState().ensureCatalogLoaded();
    // Poll delays: 500, 1000, 2000, 4000, 8000, 8000, 8000, 8000.
    // 1 initial + 8 retries = 9 catalog calls; the cap (attempt > MAX) is
    // crossed on the 9th attempt, which surfaces the error toast.
    const delays = [500, 1000, 2000, 4000, 8000, 8000, 8000, 8000];
    for (const delay of delays) {
      await vi.advanceTimersByTimeAsync(delay);
    }

    expect(mockedGetToolCatalog).toHaveBeenCalledTimes(9);
    expect(useToastStore.getState().toasts.length).toBe(1);
    expect(useToastStore.getState().toasts[0].variant).toBe("error");
  });

  it("retries a transient fetch error with backoff and does not swallow it silently", async () => {
    vi.useFakeTimers();
    mockedGetToolCatalog
      .mockRejectedValueOnce(new Error("catalog fetch failed"))
      .mockResolvedValueOnce(warmingEngines("warm"));

    void useSettingsStore.getState().ensureCatalogLoaded();
    await vi.advanceTimersByTimeAsync(500); // error → retry poll #2

    expect(mockedGetToolCatalog).toHaveBeenCalledTimes(2);
    expect(useSettingsStore.getState().engineStatuses.lsp.state).toBe("warm");
  });
});
