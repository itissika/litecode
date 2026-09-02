import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EnginesDetail } from "../../../../api/workspace";
import { EnginesSection } from "./EnginesSection";

vi.mock("../../../../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../../api/workspace")>();
  return {
    ...actual,
    getEnginesDetail: vi.fn(),
    getEngines: vi.fn(),
  };
});

vi.mock("./EngineView", () => ({
  EngineView: () => <div>engine-view</div>,
}));

import { getEngines, getEnginesDetail } from "../../../../api/workspace";

const mockedDetail = vi.mocked(getEnginesDetail);
const mockedCheap = vi.mocked(getEngines);

function warmingDetail(): EnginesDetail {
  return {
    retrieval: {
      desired: true,
      state: "warming",
      usable: "warming",
      error: null,
      model: {
        model_found: false,
        tokenizer_found: false,
        ready: false,
      },
      index: {
        status: "building",
        exists: false,
        needs_rebuild: false,
        vectors_ready: false,
        indexed_files: 0,
        indexed_chunks: 0,
      },
      policy: {
        product_internal_dirs: [],
        exclude_globs: [],
        max_file_bytes: 0,
        binary_files: false,
      },
    },
    lsp: {
      desired: true,
      state: "warming",
      usable: "warming",
      configured_servers: [],
      probes: [],
      servers: [],
    },
  };
}

describe("EnginesSection", () => {
  beforeEach(() => {
    mockedDetail.mockReset().mockResolvedValue(warmingDetail());
    mockedCheap.mockReset().mockResolvedValue({ engines: {}, lsp_servers: [] });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("hits /engines/detail only once within a warming poll interval", async () => {
    vi.useFakeTimers();
    render(<EnginesSection />);
    await Promise.resolve();
    await Promise.resolve();
    expect(mockedDetail).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(999);
    expect(mockedDetail).toHaveBeenCalledTimes(1);
  });
});
