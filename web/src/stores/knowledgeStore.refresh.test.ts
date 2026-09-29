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

import { fetchTree, readFile } from "../api/workspace";
import { useKnowledgeStore } from "./knowledgeStore";
import { useWorkspaceChangeStore } from "./workspaceChangeStore";

const tree = vi.mocked(fetchTree);
const read = vi.mocked(readFile);

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
  });
});
