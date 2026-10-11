import { describe, expect, it, vi } from "vitest";

import { KNOWLEDGE_ROOT } from "./document";
import {
  createKnowledgeFolder,
  createKnowledgeNode,
  deleteKnowledgeEntry,
  knowledgeFolderRel,
  knowledgeMoveDestination,
  knowledgeNodeRel,
} from "./load";

vi.mock("../../api/workspace", () => ({
  createFile: vi.fn(),
  mkdir: vi.fn(),
  writeFile: vi.fn(),
  deletePath: vi.fn(),
  renamePath: vi.fn(),
}));

import { createFile, deletePath, mkdir } from "../../api/workspace";

const mockedCreate = vi.mocked(createFile);
const mockedMkdir = vi.mocked(mkdir);
const mockedDelete = vi.mocked(deletePath);

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
