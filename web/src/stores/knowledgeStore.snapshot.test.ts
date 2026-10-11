import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/workspace", () => ({
  fetchTree: vi.fn(),
  readFile: vi.fn(),
  createFile: vi.fn(),
  mkdir: vi.fn(),
  writeFile: vi.fn(async () => undefined),
  deletePath: vi.fn(),
  renamePath: vi.fn(),
}));

vi.mock("../api/knowledge", () => ({
  fetchKnowledgeSnapshot: vi.fn(),
}));

import { fetchKnowledgeSnapshot, type WireKnowledgeNode, type WireKnowledgeSnapshot } from "../api/knowledge";
import { fetchTree, readFile, writeFile } from "../api/workspace";
import { knowledgeRefState, resetKnowledgeSyncForTests, useKnowledgeStore } from "./knowledgeStore";

const fetchSnapshot = vi.mocked(fetchKnowledgeSnapshot);
const tree = vi.mocked(fetchTree);
const read = vi.mocked(readFile);
const write = vi.mocked(writeFile);

const initial = useKnowledgeStore.getState();

function wireNode(partial: Partial<WireKnowledgeNode> & Pick<WireKnowledgeNode, "path" | "key">): WireKnowledgeNode {
  return {
    id: partial.key,
    status: "enabled",
    invalid_status: null,
    summary: "",
    body: "",
    x: null,
    y: null,
    w: null,
    h: null,
    extras: [],
    folder_id: null,
    relations: [],
    severity: null,
    issues: [],
    refs: [],
    ...partial,
  };
}

function snapshot(
  version: number,
  nodes: WireKnowledgeNode[],
  extra: Partial<WireKnowledgeSnapshot> = {},
): WireKnowledgeSnapshot {
  return {
    version,
    epoch: "e1",
    ready: true,
    root: "knowledge",
    visibility: "public",
    head: null,
    folders: [],
    unknown: [],
    nodes,
    ...extra,
  };
}

const missingRef = {
  ref_key: "sym:src/a.rs\u0000fn save",
  kind: "symbol" as const,
  key: null,
  path: "src/a.rs",
  symbol: "fn save",
  lines: null,
  raw: '[@ file="src/a.rs" symbol="fn save"]',
  status: "missing_symbol",
  severity: "error" as const,
  target_path: "src/a.rs",
  args: { path: "src/a.rs", symbol: "fn save" },
  message: 'Symbol "fn save" in "src/a.rs" does not exist.',
};

const seq = wireNode({
  path: "内核/seq.md",
  key: "seq",
  summary: "序号",
  body: 'See [@ file="src/a.rs" symbol="fn save"]',
  folder_id: "内核",
  severity: "error",
  issues: [
    {
      severity: "error",
      code: "missing_symbol",
      message: missingRef.message,
      ref_key: missingRef.ref_key,
      args: missingRef.args,
    },
  ],
  refs: [missingRef],
});

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

beforeEach(() => {
  resetKnowledgeSyncForTests();
  useKnowledgeStore.setState(initial, true);
  fetchSnapshot.mockReset();
  write.mockClear();
  tree.mockClear();
  read.mockClear();
});

afterEach(() => {
  resetKnowledgeSyncForTests();
});

describe("knowledgeStore backend snapshot", () => {
  it("loads the corpus and every verdict in one request, without scanning or writing", async () => {
    fetchSnapshot.mockResolvedValue(
      snapshot(3, [seq, wireNode({ path: "dup/a.md", key: "dup" }), wireNode({ path: "dup/b.md", key: "dup" })], {
        folders: [{ id: "内核", name: "内核", parent_id: null }],
        unknown: [{ path: "readme.md", name: "readme.md", folder_id: null }],
      }),
    );
    await useKnowledgeStore.getState().load();
    const state = useKnowledgeStore.getState();
    expect(fetchSnapshot).toHaveBeenCalledTimes(1);
    expect(tree).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
    expect(write).not.toHaveBeenCalled();
    // ids: unique key → key; duplicate key → path (the old knowledgeFromFiles rule).
    expect(state.nodes.map((node) => node.id)).toEqual(["seq", "dup/a.md", "dup/b.md"]);
    expect(state.byId.get("seq")?.folderId).toBe("内核");
    expect(state.root).toBe("knowledge");
    expect(state.visibility).toBe("public");
    expect(state.unknown).toEqual([{ path: "readme.md", name: "readme.md", folderId: null }]);
    expect(state.issuesByNode.get("seq")?.map((issue) => issue.code)).toEqual(["missing_symbol"]);
    expect(state.issuesByNode.get("seq")?.[0]?.refKey).toBe(missingRef.ref_key);
    expect(knowledgeRefState(state, "seq", missingRef.ref_key)?.status).toBe("missing_symbol");
    expect(knowledgeRefState(state, "seq", "sym:src/b.rs\u0000fn save")).toBeUndefined();
    expect(state.version).toBe(3);
    expect(state.ready).toBe(true);
  });

  it("refetches once per burst of newer pushes and ignores stale ones", async () => {
    fetchSnapshot.mockResolvedValue(snapshot(5, [seq]));
    await useKnowledgeStore.getState().load();
    expect(fetchSnapshot).toHaveBeenCalledTimes(1);

    useKnowledgeStore.getState().onKnowledgeChanged({ version: 5, epoch: "e1", paths: [] });
    useKnowledgeStore.getState().onKnowledgeChanged({ version: 4, epoch: "e1", paths: [] });
    await sleep(120);
    expect(fetchSnapshot).toHaveBeenCalledTimes(1);

    fetchSnapshot.mockResolvedValue(snapshot(7, [seq, wireNode({ path: "order.md", key: "order" })]));
    useKnowledgeStore.getState().onKnowledgeChanged({ version: 6, epoch: "e1", paths: ["order.md"] });
    useKnowledgeStore.getState().onKnowledgeChanged({ version: 7, epoch: "e1", paths: ["order.md"] });
    await vi.waitFor(() => expect(fetchSnapshot).toHaveBeenCalledTimes(2));
    await vi.waitFor(() =>
      expect(useKnowledgeStore.getState().nodes.map((node) => node.key)).toEqual(["seq", "order"]),
    );

    // A restarted server counts from 1 again: a new epoch is always accepted.
    fetchSnapshot.mockResolvedValue({ ...snapshot(1, [seq]), epoch: "e2" });
    useKnowledgeStore.getState().onKnowledgeChanged({ version: 1, epoch: "e2", paths: [] });
    await vi.waitFor(() => expect(useKnowledgeStore.getState().epoch).toBe("e2"));
    expect(useKnowledgeStore.getState().nodes.map((node) => node.key)).toEqual(["seq"]);
  });

  it("does not apply an older snapshot of the same epoch", async () => {
    fetchSnapshot.mockResolvedValue(snapshot(9, [seq]));
    await useKnowledgeStore.getState().load();
    fetchSnapshot.mockResolvedValue(snapshot(8, []));
    await useKnowledgeStore.getState().refreshFromDisk();
    expect(useKnowledgeStore.getState().nodes.map((node) => node.key)).toEqual(["seq"]);
    expect(useKnowledgeStore.getState().version).toBe(9);
  });

  it("pulls a full snapshot after a reconnect and keeps the last one when a request fails", async () => {
    fetchSnapshot.mockResolvedValue(snapshot(2, [seq]));
    await useKnowledgeStore.getState().load();
    fetchSnapshot.mockRejectedValue(new Error("HTTP 502"));
    useKnowledgeStore.getState().onReconnect();
    await vi.waitFor(() => expect(fetchSnapshot).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(useKnowledgeStore.getState().error).toBe("HTTP 502"));
    expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual(["missing_symbol"]);
  });

  it("marks a cold-start snapshot as not ready", async () => {
    fetchSnapshot.mockResolvedValue({
      ...snapshot(1, [{ ...seq, issues: [], severity: null, refs: [{ ...missingRef, status: "checking", severity: null, message: "" }] }]),
      ready: false,
    });
    await useKnowledgeStore.getState().load();
    const state = useKnowledgeStore.getState();
    expect(state.ready).toBe(false);
    expect(state.issues).toEqual([]);
    expect(knowledgeRefState(state, "seq", missingRef.ref_key)?.status).toBe("checking");
  });

  it("keeps a saved edit on screen until a snapshot carries it", async () => {
    fetchSnapshot.mockResolvedValue(snapshot(1, [seq]));
    await useKnowledgeStore.getState().load();

    const saved = await useKnowledgeStore.getState().saveNode("seq", { summary: "保住", value: "no citation" });
    expect(saved).toBe(true);
    expect(write).toHaveBeenCalledTimes(1);
    // The removed citation loses its issue at once.
    expect(useKnowledgeStore.getState().issuesByNode.get("seq")).toBeUndefined();

    // A push for another change: the snapshot predates the save.
    fetchSnapshot.mockResolvedValue(snapshot(2, [seq]));
    await useKnowledgeStore.getState().refreshFromDisk();
    expect(useKnowledgeStore.getState().byId.get("seq")?.summary).toBe("保住");
    expect(useKnowledgeStore.getState().byId.get("seq")?.value).toBe("no citation");
    expect(useKnowledgeStore.getState().issuesByNode.get("seq")).toBeUndefined();

    // The backend has seen the write: its snapshot wins from here on.
    fetchSnapshot.mockResolvedValue(
      snapshot(3, [{ ...seq, summary: "保住", body: "no citation", issues: [], refs: [], severity: null }]),
    );
    await useKnowledgeStore.getState().refreshFromDisk();
    fetchSnapshot.mockResolvedValue(
      snapshot(4, [{ ...seq, summary: "外部改了", body: "no citation", issues: [], refs: [], severity: null }]),
    );
    await useKnowledgeStore.getState().refreshFromDisk();
    expect(useKnowledgeStore.getState().byId.get("seq")?.summary).toBe("外部改了");
  });
});
