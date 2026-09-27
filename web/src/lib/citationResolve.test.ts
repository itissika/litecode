import { beforeEach, describe, expect, it, vi } from "vitest";

import { resolveCitations } from "../api/workspace";
import { requestCitation, resetCitationCacheForTests } from "./citationResolve";
import type { CitationTarget } from "./citationRef";

vi.mock("../api/workspace", () => ({
  resolveCitations: vi.fn(),
}));

const file = (path: string): CitationTarget => ({ kind: "file", path });

beforeEach(() => {
  resetCitationCacheForTests();
  vi.mocked(resolveCitations).mockReset();
});

describe("requestCitation", () => {
  it("coalesces refs from the same turn into one request", async () => {
    vi.mocked(resolveCitations).mockImplementation(async (refs) =>
      refs.map((ref) => ({ exists: true, path: ref.path })),
    );

    const [a, b] = await Promise.all([
      requestCitation("ws", file("src/a.ts")),
      requestCitation("ws", file("src/b.ts")),
    ]);

    expect(resolveCitations).toHaveBeenCalledTimes(1);
    expect(vi.mocked(resolveCitations).mock.calls[0][0]).toEqual([
      { path: "src/a.ts" },
      { path: "src/b.ts" },
    ]);
    expect(a).toEqual({ exists: true, path: "src/a.ts" });
    expect(b).toEqual({ exists: true, path: "src/b.ts" });
  });

  it("chunks a burst at 32 refs", async () => {
    vi.mocked(resolveCitations).mockImplementation(async (refs) =>
      refs.map((ref) => ({ exists: true, path: ref.path })),
    );
    const targets = Array.from({ length: 33 }, (_, i) => file(`src/f${i}.ts`));
    await Promise.all(targets.map((target) => requestCitation("ws", target)));
    expect(resolveCitations).toHaveBeenCalledTimes(2);
    expect(vi.mocked(resolveCitations).mock.calls[0][0]).toHaveLength(32);
    expect(vi.mocked(resolveCitations).mock.calls[1][0]).toHaveLength(1);
  });

  it("caches a hit and does not cache a miss", async () => {
    vi.mocked(resolveCitations)
      .mockResolvedValueOnce([{ exists: true, path: "src/a.ts", line: 4 }])
      .mockResolvedValueOnce([{ exists: false }])
      .mockResolvedValueOnce([{ exists: true, path: "src/missing.ts" }]);

    await requestCitation("ws", { kind: "line", path: "src/a.ts", line: 4 });
    await requestCitation("ws", { kind: "line", path: "src/a.ts", line: 4 });
    await requestCitation("ws", file("src/missing.ts"));
    await requestCitation("ws", file("src/missing.ts"));

    expect(resolveCitations).toHaveBeenCalledTimes(3);
    expect(vi.mocked(resolveCitations).mock.calls[1][0]).toEqual([
      { path: "src/missing.ts" },
    ]);
    expect(vi.mocked(resolveCitations).mock.calls[2][0]).toEqual([
      { path: "src/missing.ts" },
    ]);
  });

  it("sends a line or a symbol, not both", async () => {
    vi.mocked(resolveCitations).mockResolvedValue([
      { exists: true, path: "src/a.ts", line: 3 },
    ]);
    await requestCitation("ws", {
      kind: "symbol",
      path: "src/a.ts",
      symbol: "Session.user",
    });
    expect(vi.mocked(resolveCitations).mock.calls[0][0]).toEqual([
      { path: "src/a.ts", symbol: "Session.user" },
    ]);
  });
});
