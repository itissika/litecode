import { afterEach, describe, expect, it, vi } from "vitest";

import {
  fetchGlob,
  fetchTreeReveal,
  getEnginesDetail,
  gitStatus,
  refreshRetrieval,
} from "./workspace";

describe("workspace engine API", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
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

  it("fetches git status from the workspace git endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        ok: true,
        data: {
          is_repo: true,
          branch: "main",
          upstream_ahead: 0,
          upstream_behind: 0,
          staged: [],
          changes: [],
        },
      }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const status = await gitStatus();
    expect(status.branch).toBe("main");
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/workspace/git/status");
  });

  it("fetches filename glob hits from the workspace glob endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        ok: true,
        data: {
          entries: [{ name: "FileTree.tsx", path: "src/FileTree.tsx", kind: "file" }],
          truncated: false,
        },
      }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const listing = await fetchGlob("FileTree");
    expect(listing.entries).toEqual([
      { name: "FileTree.tsx", path: "src/FileTree.tsx", kind: "file" },
    ]);
    expect(listing.truncated).toBe(false);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/workspace/glob?pattern=FileTree",
    );
  });

  it("fetches tree reveal ancestor listings", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        ok: true,
        data: {
          by_dir: {
            "": [{ name: "src", path: "src", kind: "dir" }],
            src: [{ name: "a.ts", path: "src/a.ts", kind: "file" }],
          },
        },
      }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const byDir = await fetchTreeReveal("src/a.ts");
    expect(byDir.src).toEqual([{ name: "a.ts", path: "src/a.ts", kind: "file" }]);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/workspace/tree?path=src%2Fa.ts&reveal=1",
    );
  });
});
