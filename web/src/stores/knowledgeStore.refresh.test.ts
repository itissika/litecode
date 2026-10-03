import { describe, expect, it, vi } from "vitest";

vi.mock("../api/workspace", () => ({
  fetchTree: vi.fn(),
  readFile: vi.fn(),
  createFile: vi.fn(),
  mkdir: vi.fn(),
  writeFile: vi.fn(),
  deletePath: vi.fn(),
  renamePath: vi.fn(),
  resolveSymbolRefs: vi.fn(async () => []),
}));

import { fetchTree, readFile, resolveSymbolRefs, writeFile } from "../api/workspace";
import { symbolHitFor, useKnowledgeStore } from "./knowledgeStore";
import { useWorkspaceChangeStore } from "./workspaceChangeStore";

const tree = vi.mocked(fetchTree);
const read = vi.mocked(readFile);
const write = vi.mocked(writeFile);
const resolve = vi.mocked(resolveSymbolRefs);

const seq = [
  "```node",
  "node : seq",
  "status : enabled",
  "summary : 序号",
  "```",
  "",
  "正文",
  "",
].join("\n");

const order = [
  "```node",
  "node : order",
  "status : pending",
  "summary : ",
  "```",
  "",
  "",
].join("\n");

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

describe("knowledgeStore workspace refresh", () => {
  it("reloads after a debounced change inside the knowledge root", async () => {
    const files = ["seq.md"];
    tree.mockImplementation(async (dir = "") => {
      if (dir === "knowledge") throw new Error("missing");
      if (dir !== ".litecode/knowledge") throw new Error(dir);
      return files.map((name) => ({
        name,
        path: `.litecode/knowledge/${name}`,
        kind: "file" as const,
      }));
    });
    read.mockImplementation(async (path: string) => {
      if (path.endsWith("/order.md")) return order;
      return seq;
    });

    await useKnowledgeStore.getState().load();
    await useKnowledgeStore.getState().notePanelVisible("workspace-knowledge", true);
    expect(useKnowledgeStore.getState().nodes.map((node) => node.key)).toEqual([
      "seq",
    ]);

    const before = tree.mock.calls.length;
    useWorkspaceChangeStore.getState().record(["src/main.rs"], "modified");
    await sleep(200);
    expect(tree.mock.calls.length).toBe(before);
    expect(resolve.mock.calls.length).toBe(0);

    files.push("order.md");
    useWorkspaceChangeStore.getState().record(
      [".litecode/knowledge/order.md"],
      "created",
    );
    useWorkspaceChangeStore
      .getState()
      .record([".litecode/knowledge/seq.md"], "modified");
    await sleep(80);
    expect(tree.mock.calls.length).toBe(before);
    await sleep(150);
    expect(tree.mock.calls.length).toBe(before + 3);
    expect(useKnowledgeStore.getState().nodes.map((node) => node.key).sort()).toEqual(
      ["order", "seq"],
    );
    await useKnowledgeStore.getState().notePanelVisible("workspace-knowledge", false);
  });

  it("does not scan disk while every knowledge surface is hidden", async () => {
    tree.mockImplementation(async (dir = "") => {
      if (dir === "knowledge") throw new Error("missing");
      if (dir !== ".litecode/knowledge") throw new Error(dir);
      return [
        {
          name: "seq.md",
          path: ".litecode/knowledge/seq.md",
          kind: "file" as const,
        },
      ];
    });
    read.mockResolvedValue(seq);
    await useKnowledgeStore.getState().load();
    useKnowledgeStore.getState().notePanelVisible("workspace-knowledge", false);
    useKnowledgeStore.getState().notePanelVisible("knowledge-graph", false);
    const before = tree.mock.calls.length;
    useWorkspaceChangeStore
      .getState()
      .record([".litecode/knowledge/seq.md"], "modified");
    await sleep(200);
    expect(tree.mock.calls.length).toBe(before);
  });

  it("keeps a single-card edit when a disk snapshot started earlier", async () => {
    let disk = seq;
    let holdNextRead = false;
    let releaseRead: () => void = () => {};
    let enteredRead: () => void = () => {};
    const readEntered = new Promise<void>((resolve) => {
      enteredRead = resolve;
    });
    tree.mockImplementation(async (dir = "") => {
      if (dir === "knowledge") throw new Error("missing");
      if (dir !== ".litecode/knowledge") throw new Error(dir);
      return [
        {
          name: "seq.md",
          path: ".litecode/knowledge/seq.md",
          kind: "file" as const,
        },
      ];
    });
    read.mockImplementation(async () => {
      if (holdNextRead) {
        holdNextRead = false;
        const snap = disk;
        enteredRead();
        await new Promise<void>((resolve) => {
          releaseRead = resolve;
        });
        return snap;
      }
      return disk;
    });
    write.mockImplementation(async (_path: string, content: string) => {
      disk = content;
    });

    await useKnowledgeStore.getState().load();
    const id = useKnowledgeStore.getState().nodes[0]?.id;
    expect(id).toBeTruthy();

    holdNextRead = true;
    const refresh = useKnowledgeStore.getState().refreshFromDisk();
    await readEntered;
    const saved = await useKnowledgeStore.getState().saveNode(id!, { summary: "保住" });
    expect(saved).toBe(true);
    expect(useKnowledgeStore.getState().byId.get(id!)?.summary).toBe("保住");

    releaseRead();
    await refresh;
    await sleep(30);
    expect(useKnowledgeStore.getState().byId.get(id!)?.summary).toBe("保住");
  });

  it("pairs a symbol hit by file and symbol", () => {
    const hits = [
      { file: "src/b.rs", symbol: "fn other", file_exists: true, symbol_exists: true, ambiguous: false },
      { file: "src/a.rs", symbol: "fn save", file_exists: true, symbol_exists: true, ambiguous: false, drift: { drifted: true } },
    ];
    expect(symbolHitFor(hits, "src/a.rs", "fn save")?.drift?.drifted).toBe(true);
    expect(symbolHitFor(hits, "src/a.rs", "fn missing")).toBeUndefined();
  });

  it("updates error and warning counts when a cited file or HEAD changes", async () => {
    const cited = [
      "```node",
      "node : seq",
      "status : enabled",
      "summary : ",
      "```",
      "",
      'See [@ file="src/a.rs" symbol="fn save"]',
      "",
    ].join("\n");
    let srcNames = ["a.rs"];
    tree.mockImplementation(async (dir = "") => {
      if (dir === "src") {
        return srcNames.map((name) => ({
          name,
          path: `src/${name}`,
          kind: "file" as const,
        }));
      }
      if (dir === "knowledge") throw new Error("missing");
      if (dir !== ".litecode/knowledge") throw new Error(dir);
      return [
        {
          name: "seq.md",
          path: ".litecode/knowledge/seq.md",
          kind: "file" as const,
        },
      ];
    });
    read.mockResolvedValue(cited);
    resolve.mockResolvedValue([
      {
        file: "src/a.rs",
        symbol: "fn save",
        file_exists: true,
        symbol_exists: true,
        ambiguous: false,
        drift: { drifted: false },
      },
    ]);

    await useKnowledgeStore.getState().load();
    useKnowledgeStore.getState().notePanelVisible("workspace-knowledge", false);
    useKnowledgeStore.getState().notePanelVisible("knowledge-graph", false);
    await vi.waitFor(() => {
      expect(resolve.mock.calls.length).toBeGreaterThan(0);
    });
    expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([]);

    const knowledgeReads = () =>
      tree.mock.calls.filter((call) => String(call[0]).includes("knowledge")).length;
    const before = knowledgeReads();
    const beforeResolve = resolve.mock.calls.length;

    srcNames = [];
    useWorkspaceChangeStore.getState().record(["src/a.rs"], "deleted");
    await sleep(80);
    expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([]);
    await vi.waitFor(() => {
      expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([
        "missing_file",
      ]);
    });
    expect(knowledgeReads()).toBe(before);

    srcNames = ["a.rs"];
    resolve.mockResolvedValue([
      {
        file: "src/a.rs",
        symbol: "fn save",
        file_exists: true,
        symbol_exists: true,
        ambiguous: false,
        drift: { drifted: true },
      },
    ]);
    useWorkspaceChangeStore.getState().record(["src/a.rs"], "modified");
    await vi.waitFor(() => {
      expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([
        "symbol_drift",
      ]);
    });

    resolve.mockResolvedValue([
      {
        file: "src/a.rs",
        symbol: "fn save",
        file_exists: true,
        symbol_exists: true,
        ambiguous: false,
        drift: { drifted: false },
      },
    ]);
    useWorkspaceChangeStore.getState().recordHead();
    await vi.waitFor(() => {
      expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([]);
    });
    expect(resolve.mock.calls.length).toBeGreaterThan(beforeResolve);
    expect(knowledgeReads()).toBe(before);

    resolve.mockResolvedValue([
      {
        file: "src/a.rs",
        symbol: "fn save",
        file_exists: true,
        symbol_exists: true,
        ambiguous: false,
        drift: { drifted: true },
      },
    ]);
    useWorkspaceChangeStore.getState().record(["src/a.rs"], "modified");
    await vi.waitFor(() => {
      expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([
        "symbol_drift",
      ]);
    });

    const id = useKnowledgeStore.getState().nodes[0]?.id;
    const calls = resolve.mock.calls.length;
    const saved = await useKnowledgeStore.getState().saveNode(id!, { value: "正文\n" });
    expect(saved).toBe(true);
    expect(useKnowledgeStore.getState().issues.map((issue) => issue.code)).toEqual([]);
    expect(resolve.mock.calls.length).toBe(calls);
  });
});
