import { describe, expect, it, vi } from "vitest";

vi.mock("../api/workspace", () => ({
  fetchTree: vi.fn(),
  readFile: vi.fn(),
  createFile: vi.fn(),
  mkdir: vi.fn(),
  writeFile: vi.fn(),
  deletePath: vi.fn(),
  renamePath: vi.fn(),
}));

import { fetchTree, readFile, writeFile } from "../api/workspace";
import { symbolHitFor, useKnowledgeStore } from "./knowledgeStore";
import { useWorkspaceChangeStore } from "./workspaceChangeStore";

const tree = vi.mocked(fetchTree);
const read = vi.mocked(readFile);
const write = vi.mocked(writeFile);

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
      { file: "src/a.rs", symbol: "fn save", file_exists: true, symbol_exists: true, ambiguous: false, drift: { drifted: true, commits: [] } },
    ];
    expect(symbolHitFor(hits, "src/a.rs", "fn save")?.drift?.drifted).toBe(true);
    expect(symbolHitFor(hits, "src/a.rs", "fn missing")).toBeUndefined();
  });
});
