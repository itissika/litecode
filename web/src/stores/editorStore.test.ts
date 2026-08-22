import { beforeEach, describe, expect, it, vi } from "vitest";

import { useEditorStore } from "./editorStore";
import { readFile, writeFile } from "../api/workspace";

vi.mock("../api/workspace", () => ({
  readFile: vi.fn(),
  writeFile: vi.fn(),
}));

const mockedReadFile = vi.mocked(readFile);
const mockedWriteFile = vi.mocked(writeFile);

function tabState(path: string, dirty: boolean) {
  useEditorStore.setState({
    tabs: [
      {
        path,
        content: dirty ? "unsaved-edit" : "clean",
        savedContent: "clean",
        dirty,
        language: "typescript",
        loading: false,
        error: null,
      },
    ],
    conflicts: {},
    activePath: path,
  });
}

beforeEach(() => {
  useEditorStore.setState({
    tabs: [],
    conflicts: {},
    activePath: null,
    saving: false,
    mdViewByPath: {},
  });
  mockedReadFile.mockReset();
  mockedWriteFile.mockReset();
});

describe("handleWorkspaceChange agent-first disk authority", () => {
  it("reloads a dirty tab from disk instead of recording a conflict", async () => {
    const path = "src/a.ts";
    tabState(path, true);
    mockedReadFile.mockResolvedValue("agent-wrote");

    await useEditorStore.getState().handleWorkspaceChange([path], "modified");

    expect(useEditorStore.getState().conflicts[path]).toBeUndefined();
    expect(mockedReadFile).toHaveBeenCalledWith(path);
    const tab = useEditorStore.getState().tabs.find((t) => t.path === path)!;
    expect(tab.content).toBe("agent-wrote");
    expect(tab.dirty).toBe(false);
  });

  it("reloads a clean tab from disk and records no conflict", async () => {
    const path = "src/clean.ts";
    tabState(path, false);
    mockedReadFile.mockResolvedValue("clean");

    await useEditorStore.getState().handleWorkspaceChange([path], "modified");

    expect(useEditorStore.getState().conflicts[path]).toBeUndefined();
    expect(mockedReadFile).toHaveBeenCalledWith(path);
  });

  it("clears a conflict via clearConflict", () => {
    const path = "src/a.ts";
    useEditorStore.setState({
      tabs: [],
      conflicts: { [path]: { path, source: "agent" } },
    });
    useEditorStore.getState().clearConflict(path);
    expect(useEditorStore.getState().conflicts[path]).toBeUndefined();
  });
});

describe("save content snapshot", () => {
  it("keeps dirty true when content changes during an in-flight save", async () => {
    const path = "src/a.ts";
    let resolveWrite!: () => void;
    mockedWriteFile.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveWrite = resolve;
        }),
    );
    useEditorStore.setState({
      tabs: [
        {
          path,
          content: "A",
          savedContent: "A",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
        },
      ],
      activePath: path,
      conflicts: {},
      saving: false,
    });

    const savePromise = useEditorStore.getState().save(path);
    useEditorStore.getState().setContent(path, "B");
    resolveWrite();
    await savePromise;

    expect(mockedWriteFile).toHaveBeenCalledWith(path, "A");
    const tab = useEditorStore.getState().tabs.find((t) => t.path === path)!;
    expect(tab.savedContent).toBe("A");
    expect(tab.content).toBe("B");
    expect(tab.dirty).toBe(true);
  });
});

describe("remapTabs on rename", () => {
  it("rewrites open tab paths including descendants and keeps dirty buffers", () => {
    useEditorStore.setState({
      tabs: [
        {
          path: "src/a.ts",
          content: "unsaved",
          savedContent: "clean",
          dirty: true,
          language: "typescript",
          loading: false,
          error: null,
        },
        {
          path: "src/a/inner.ts",
          content: "x",
          savedContent: "x",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
        },
        {
          path: "other.ts",
          content: "y",
          savedContent: "y",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
        },
      ],
      activePath: "src/a.ts",
      conflicts: {},
    });

    useEditorStore.getState().remapTabs("src/a", "src/b");

    const tabs = useEditorStore.getState().tabs;
    expect(tabs.map((t) => t.path).sort()).toEqual([
      "other.ts",
      "src/a.ts",
      "src/b/inner.ts",
    ]);
    expect(useEditorStore.getState().activePath).toBe("src/a.ts");

    useEditorStore.getState().remapTabs("src/a.ts", "src/c.ts");
    const moved = useEditorStore.getState().tabs.find((t) => t.path === "src/c.ts")!;
    expect(moved.content).toBe("unsaved");
    expect(moved.dirty).toBe(true);
    expect(useEditorStore.getState().activePath).toBe("src/c.ts");
  });

  it("ignores stale delete after remap", async () => {
    useEditorStore.setState({
      tabs: [
        {
          path: "src/b.ts",
          content: "ok",
          savedContent: "ok",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
        },
      ],
      activePath: "src/b.ts",
      conflicts: {},
    });
    await useEditorStore.getState().handleWorkspaceChange(["src/a.ts"], "deleted");
    expect(useEditorStore.getState().tabs).toHaveLength(1);
    expect(useEditorStore.getState().tabs[0]?.path).toBe("src/b.ts");
  });
});

describe("markdown editor view", () => {
  it("forces source when opening a markdown file at a line", async () => {
    mockedReadFile.mockResolvedValue("# hi\n");
    await useEditorStore.getState().openFileAt("docs/readme.md", 2);
    expect(useEditorStore.getState().mdViewByPath["docs/readme.md"]).toBe(
      "source",
    );
    expect(useEditorStore.getState().pendingReveal).toEqual({
      path: "docs/readme.md",
      line: 2,
    });
  });
});
