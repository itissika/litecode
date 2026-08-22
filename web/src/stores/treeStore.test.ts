import { beforeEach, describe, expect, it, vi } from "vitest";

import { useTreeStore } from "./treeStore";
import { fetchTree, type TreeEntry } from "../api/workspace";

vi.mock("../api/workspace", () => ({
  fetchTree: vi.fn(),
}));

const mockedFetchTree = vi.mocked(fetchTree);

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
});

describe("revealPath", () => {
  it("expands every ancestor so the file becomes visible", async () => {
    mockedFetchTree.mockResolvedValue([]);
    await useTreeStore.getState().revealPath("src/components/FileTree.tsx");

    expect(useTreeStore.getState().expanded.has("src")).toBe(true);
    expect(useTreeStore.getState().expanded.has("src/components")).toBe(true);
    // The file itself and the workspace root are not dirs to expand.
    expect(useTreeStore.getState().expanded.has("")).toBe(false);
    expect(
      useTreeStore.getState().expanded.has("src/components/FileTree.tsx"),
    ).toBe(false);
  });

  it("loads children of ancestors lazily when not fetched yet", async () => {
    mockedFetchTree.mockImplementation(async (path) => {
      if (path === "src") return [dir("src/components", "components")];
      if (path === "src/components") return [file("src/components/a.ts", "a.ts")];
      return [];
    });

    await useTreeStore.getState().revealPath("src/components/a.ts");

    expect(mockedFetchTree).toHaveBeenCalledWith("src", 1);
    expect(mockedFetchTree).toHaveBeenCalledWith("src/components", 1);
    expect(useTreeStore.getState().children["src"]).toEqual([
      dir("src/components", "components"),
    ]);
  });

  it("does nothing for the workspace root or empty paths", async () => {
    await useTreeStore.getState().revealPath("");
    expect(useTreeStore.getState().expanded.size).toBe(0);
    expect(mockedFetchTree).not.toHaveBeenCalled();
  });
});
