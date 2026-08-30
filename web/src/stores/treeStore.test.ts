import { beforeEach, describe, expect, it, vi } from "vitest";

import { useTreeStore } from "./treeStore";
import { fetchTree, fetchTreeReveal, type TreeEntry } from "../api/workspace";

vi.mock("../api/workspace", () => ({
  fetchTree: vi.fn(),
  fetchTreeReveal: vi.fn(),
}));

const mockedFetchTree = vi.mocked(fetchTree);
const mockedFetchTreeReveal = vi.mocked(fetchTreeReveal);

const dir = (path: string, name: string): TreeEntry => ({ path, name, kind: "dir" });
const file = (path: string, name: string): TreeEntry => ({ path, name, kind: "file" });

beforeEach(() => {
  useTreeStore.setState({
    children: {},
    expanded: new Set<string>(),
    loading: new Set<string>(),
    error: null,
  });
  mockedFetchTree.mockReset();
  mockedFetchTreeReveal.mockReset();
});

describe("revealPath", () => {
  it("expands every ancestor so the file becomes visible", async () => {
    mockedFetchTreeReveal.mockResolvedValue({
      "": [],
      src: [],
      "src/components": [],
    });
    await useTreeStore.getState().revealPath("src/components/FileTree.tsx");

    expect(useTreeStore.getState().expanded.has("src")).toBe(true);
    expect(useTreeStore.getState().expanded.has("src/components")).toBe(true);
    // The file itself and the workspace root are not dirs to expand.
    expect(useTreeStore.getState().expanded.has("")).toBe(false);
    expect(
      useTreeStore.getState().expanded.has("src/components/FileTree.tsx"),
    ).toBe(false);
  });

  it("loads children of ancestors in one reveal request", async () => {
    mockedFetchTreeReveal.mockResolvedValue({
      "": [dir("src", "src")],
      src: [dir("src/components", "components")],
      "src/components": [file("src/components/a.ts", "a.ts")],
    });

    await useTreeStore.getState().revealPath("src/components/a.ts");

    expect(mockedFetchTreeReveal).toHaveBeenCalledTimes(1);
    expect(mockedFetchTreeReveal).toHaveBeenCalledWith("src/components/a.ts");
    expect(mockedFetchTree).not.toHaveBeenCalled();
    expect(useTreeStore.getState().children["src"]).toEqual([
      dir("src/components", "components"),
    ]);
  });

  it("does nothing for the workspace root or empty paths", async () => {
    await useTreeStore.getState().revealPath("");
    expect(useTreeStore.getState().expanded.size).toBe(0);
    expect(mockedFetchTreeReveal).not.toHaveBeenCalled();
    expect(mockedFetchTree).not.toHaveBeenCalled();
  });
});
