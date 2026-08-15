import { afterEach, describe, expect, it, vi } from "vitest";

import {
  clearLspServers,
  getEnginesDetail,
  initRetrieval,
  refreshRetrieval,
  stopLsp,
  stopRetrieval,
} from "./workspace";

describe("workspace engine API", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses dedicated lifecycle endpoints for workspace engines", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true, data: { desired: true } }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await initRetrieval();
    await stopRetrieval();
    await stopLsp();
    await clearLspServers();

    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/api/workspace/retrieval/init",
      "/api/workspace/retrieval/stop",
      "/api/workspace/lsp/stop",
      "/api/workspace/lsp/clear",
    ]);
    expect(fetchMock.mock.calls.every(([, init]) => init?.method === "POST")).toBe(true);
  });

  it("posts retrieval refresh and returns mode", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true, data: { desired: true, mode: "incremental" } }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await refreshRetrieval();
    expect(result.mode).toBe("incremental");
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/workspace/retrieval/refresh");
    expect(fetchMock.mock.calls[0]?.[1]?.method).toBe("POST");
  });

  it("parses native retrieval and LSP detail payloads", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        ok: true,
        data: {
          retrieval: {
            desired: true,
            usable: "ready",
            model: { ready: true },
            index: {
              status: "ready",
              indexed_files: 1,
              indexed_chunks: 2,
              progress: null,
            },
            policy: {},
          },
          lsp: { desired: false, usable: "stopped", configured_servers: [], probes: [] },
        },
      }),
    }));
    const detail = await getEnginesDetail();
    expect(detail.retrieval.usable).toBe("ready");
    expect(detail.retrieval.index.status).toBe("ready");
    expect(detail.retrieval.index.indexed_chunks).toBe(2);
    expect(detail.lsp.usable).toBe("stopped");
  });
});
