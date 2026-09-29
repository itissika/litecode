import { describe, expect, it, vi } from "vitest";

import { KNOWLEDGE_ROOT } from "./document";
import {
  KNOWLEDGE_PUBLIC_ROOT,
  createKnowledgeFolder,
  createKnowledgeNode,
  deleteKnowledgeEntry,
  knowledgeFolderRel,
  knowledgeMoveDestination,
  knowledgeNodeRel,
  loadKnowledgeFromWorkspace,
} from "./load";

vi.mock("../../api/workspace", () => ({
  fetchTree: vi.fn(),
  readFile: vi.fn(),
  createFile: vi.fn(),
  mkdir: vi.fn(),
  writeFile: vi.fn(),
  deletePath: vi.fn(),
  renamePath: vi.fn(),
}));

import {
  createFile,
  deletePath,
  fetchTree,
  mkdir,
  readFile,
} from "../../api/workspace";

const mockedTree = vi.mocked(fetchTree);
const mockedRead = vi.mocked(readFile);
const mockedCreate = vi.mocked(createFile);
const mockedMkdir = vi.mocked(mkdir);
const mockedDelete = vi.mocked(deletePath);

const seqMarkdown = [
  "```node",
  "node : seq",
  "status : enabled",
  "summary : 序号",
  "```",
  "",
  "正文",
  "",
].join("\n");

describe("loadKnowledgeFromWorkspace", () => {
  it("leaves an empty private corpus when neither location exists", async () => {
    mockedCreate.mockClear();
    mockedMkdir.mockClear();
    mockedTree.mockImplementation(async () => {
      throw new Error("not found");
    });

    const loaded = await loadKnowledgeFromWorkspace();
    expect(mockedMkdir).not.toHaveBeenCalled();
    expect(mockedCreate).not.toHaveBeenCalled();
    expect(loaded.visibility).toBe("private");
    expect(loaded.root).toBe(KNOWLEDGE_ROOT);
    expect(loaded.nodes).toEqual([]);
    expect(loaded.unknown).toEqual([]);
  });

  it("reads a public knowledge directory without creating files", async () => {
    mockedCreate.mockClear();
    mockedMkdir.mockClear();
    mockedTree.mockImplementation(async (dir = "") => {
      if (dir === KNOWLEDGE_PUBLIC_ROOT) {
        return [
          { name: "seq.md", path: `${KNOWLEDGE_PUBLIC_ROOT}/seq.md`, kind: "file" },
          { name: "notes.md", path: `${KNOWLEDGE_PUBLIC_ROOT}/notes.md`, kind: "file" },
        ];
      }
      throw new Error("not found");
    });
    mockedRead.mockImplementation(async (path: string) => {
      if (path.endsWith("/notes.md")) return "plain note\n";
      return seqMarkdown;
    });

    const loaded = await loadKnowledgeFromWorkspace();
    expect(mockedCreate).not.toHaveBeenCalled();
    expect(mockedMkdir).not.toHaveBeenCalled();
    expect(loaded.visibility).toBe("public");
    expect(loaded.nodes.map((node) => node.key)).toEqual(["seq"]);
    expect(loaded.unknown).toEqual([
      { path: "notes.md", name: "notes.md", folderId: null },
    ]);
  });
});

describe("knowledge create and delete paths", () => {
  it("rejects a folder name that is not a single path segment", () => {
    expect(knowledgeFolderRel("内核", "a/b")).toBeNull();
    expect(knowledgeFolderRel(null, "..")).toBeNull();
    expect(knowledgeFolderRel("内核", "上下文")).toBe("内核/上下文");
    expect(knowledgeMoveDestination("knowledge入门/文件夹关系", null)).toBe(
      "文件夹关系",
    );
    expect(knowledgeMoveDestination("note.md", "knowledge入门")).toBe(
      "knowledge入门/note.md",
    );
  });

  it("maps a declaration onto key.md", () => {
    expect(knowledgeNodeRel(null, " seq ")).toBe("seq.md");
    expect(knowledgeNodeRel("内核", "a/b")).toBeNull();
    expect(knowledgeNodeRel(null, "knowledge 概念概述")).toBe(
      "knowledge 概念概述.md",
    );
  });

  it("creates the directory, the md, and deletes either", async () => {
    mockedMkdir.mockClear();
    mockedCreate.mockClear();
    mockedDelete.mockClear();
    mockedMkdir.mockResolvedValue(`${KNOWLEDGE_ROOT}/草稿`);
    mockedCreate.mockResolvedValue(undefined);
    mockedDelete.mockResolvedValue(undefined);

    await createKnowledgeFolder(KNOWLEDGE_ROOT, "草稿");
    await createKnowledgeNode(KNOWLEDGE_ROOT, "草稿/note.md", "note");
    await deleteKnowledgeEntry(KNOWLEDGE_ROOT, "草稿/note.md", false);
    await deleteKnowledgeEntry(KNOWLEDGE_ROOT, "草稿", true);

    expect(mockedMkdir).toHaveBeenCalledWith(`${KNOWLEDGE_ROOT}/草稿`);
    expect(mockedCreate).toHaveBeenCalledWith(
      `${KNOWLEDGE_ROOT}/草稿/note.md`,
      expect.stringContaining("node : note"),
    );
    expect(mockedDelete).toHaveBeenCalledWith(
      `${KNOWLEDGE_ROOT}/草稿/note.md`,
      false,
    );
    expect(mockedDelete).toHaveBeenCalledWith(`${KNOWLEDGE_ROOT}/草稿`, true);
  });
});
