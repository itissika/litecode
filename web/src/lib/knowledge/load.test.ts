import { describe, expect, it, vi } from "vitest";

import type { TreeEntry } from "../../api/workspace";
import { KNOWLEDGE_ROOT } from "./document";
import {
  KNOWLEDGE_PRIVATE_ROOT,
  KNOWLEDGE_PUBLIC_ROOT,
  createKnowledgeFolder,
  createKnowledgeNode,
  deleteKnowledgeEntry,
  knowledgeFolderRel,
  knowledgeMoveDestination,
  knowledgeNodeRel,
  loadKnowledgeFromWorkspace,
} from "./load";
import { knowledgeSeedFiles } from "./seed";

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

function entriesUnder(root: string, dir: string): TreeEntry[] {
  const prefix = `${dir.replace(/\/$/, "")}/`;
  const childDirs = new Set<string>();
  const files: TreeEntry[] = [];
  for (const file of knowledgeSeedFiles) {
    const full = `${root}/${file.path}`;
    if (!full.startsWith(prefix)) continue;
    const rel = full.slice(prefix.length);
    const slash = rel.indexOf("/");
    if (slash === -1) {
      files.push({ name: rel, path: full, kind: "file" });
    } else {
      childDirs.add(rel.slice(0, slash));
    }
  }
  return [
    ...[...childDirs].map((name) => ({
      name,
      path: `${prefix}${name}`,
      kind: "dir" as const,
    })),
    ...files,
  ];
}

describe("loadKnowledgeFromWorkspace", () => {
  it("seeds the private tree when neither location exists", async () => {
    mockedCreate.mockClear();
    mockedMkdir.mockClear();
    mockedCreate.mockResolvedValue(undefined);
    mockedMkdir.mockResolvedValue(KNOWLEDGE_PRIVATE_ROOT);
    mockedRead.mockImplementation(async (path: string) => {
      const rel = path.slice(`${KNOWLEDGE_PRIVATE_ROOT}/`.length);
      const file = knowledgeSeedFiles.find((item) => item.path === rel);
      if (!file) throw new Error(`missing ${path}`);
      return file.markdown;
    });
    let misses = 2;
    mockedTree.mockImplementation(async (dir = "") => {
      if (misses > 0) {
        misses -= 1;
        throw new Error("not found");
      }
      return entriesUnder(KNOWLEDGE_PRIVATE_ROOT, dir);
    });

    const loaded = await loadKnowledgeFromWorkspace();
    expect(mockedMkdir).toHaveBeenCalledWith(KNOWLEDGE_PRIVATE_ROOT);
    expect(mockedCreate).toHaveBeenCalledTimes(knowledgeSeedFiles.length);
    expect(mockedCreate).toHaveBeenCalledWith(
      `${KNOWLEDGE_PRIVATE_ROOT}/knowledge入门/文件夹关系/knowledge 概念概述.md`,
      expect.stringContaining("node : knowledge 概念概述"),
    );
    expect(loaded.visibility).toBe("private");
    expect(loaded.root).toBe(KNOWLEDGE_ROOT);
    expect(loaded.nodes).toHaveLength(knowledgeSeedFiles.length);
    expect(
      loaded.nodes.find((node) => node.key === "knowledge 概念概述")?.folderId,
    ).toBe("knowledge入门/文件夹关系");
  });

  it("reads a public knowledge directory without seeding", async () => {
    mockedCreate.mockClear();
    mockedTree.mockImplementation(async (dir = "") => {
      if (dir === KNOWLEDGE_PUBLIC_ROOT || dir.startsWith(`${KNOWLEDGE_PUBLIC_ROOT}/`)) {
        return entriesUnder(KNOWLEDGE_PUBLIC_ROOT, dir);
      }
      throw new Error("not found");
    });
    mockedRead.mockImplementation(async (path: string) => {
      const rel = path.slice(`${KNOWLEDGE_PUBLIC_ROOT}/`.length);
      return knowledgeSeedFiles.find((item) => item.path === rel)?.markdown ?? "";
    });

    const loaded = await loadKnowledgeFromWorkspace();
    expect(mockedCreate).not.toHaveBeenCalled();
    expect(loaded.visibility).toBe("public");
    expect(loaded.nodes.map((node) => node.key)).toContain("knowledge agent 入门");
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
