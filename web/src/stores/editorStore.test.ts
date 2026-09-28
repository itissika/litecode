import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DockviewApi } from "dockview-react";

import { WorkspaceRequestError } from "../lib/workspaceError";
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
        errorRetryable: false,
        kind: "text" as const,
        diskRevision: 0,
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
        errorRetryable: false,
        kind: "text" as const,
        diskRevision: 0,
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
        errorRetryable: false,
        kind: "text" as const,
        diskRevision: 0,
        },
        {
          path: "src/a/inner.ts",
          content: "x",
          savedContent: "x",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
        errorRetryable: false,
        kind: "text" as const,
        diskRevision: 0,
        },
        {
          path: "other.ts",
          content: "y",
          savedContent: "y",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
        errorRetryable: false,
        kind: "text" as const,
        diskRevision: 0,
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
    const moved = useEditorStore
      .getState()
      .tabs.find((t) => t.path === "src/c.ts")!;
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
        errorRetryable: false,
        kind: "text" as const,
        diskRevision: 0,
        },
      ],
      activePath: "src/b.ts",
      conflicts: {},
    });
    await useEditorStore
      .getState()
      .handleWorkspaceChange(["src/a.ts"], "deleted");
    expect(useEditorStore.getState().tabs).toHaveLength(1);
    expect(useEditorStore.getState().tabs[0]?.path).toBe("src/b.ts");
  });
});

describe("ensureReadable", () => {
  it("does not reload a dirty tab", async () => {
    const path = "src/a.ts";
    tabState(path, true);
    mockedReadFile.mockResolvedValue("from-disk");

    await useEditorStore.getState().ensureReadable(path);

    expect(mockedReadFile).not.toHaveBeenCalled();
    expect(useEditorStore.getState().tabs[0]?.content).toBe("unsaved-edit");
  });

  it("reloads a clean text tab when disk content changed", async () => {
    const path = "src/clean.ts";
    tabState(path, false);
    mockedReadFile.mockResolvedValue("newer");

    await useEditorStore.getState().ensureReadable(path);

    const tab = useEditorStore.getState().tabs[0]!;
    expect(tab.content).toBe("newer");
    expect(tab.dirty).toBe(false);
    expect(tab.error).toBeNull();
  });

  it("retries a retryable failure and keeps a permanent one", async () => {
    const path = "src/a.ts";
    tabState(path, false);
    useEditorStore.setState({
      tabs: [
        {
          ...useEditorStore.getState().tabs[0]!,
          content: "",
          savedContent: "",
          error: "offline",
          errorRetryable: true,
        },
      ],
    });
    mockedReadFile.mockResolvedValue("back");
    await useEditorStore.getState().ensureReadable(path);
    expect(useEditorStore.getState().tabs[0]?.content).toBe("back");

    useEditorStore.setState({
      tabs: [
        {
          ...useEditorStore.getState().tabs[0]!,
          content: "",
          savedContent: "",
          error: "missing",
          errorRetryable: false,
        },
      ],
    });
    mockedReadFile.mockClear();
    await useEditorStore.getState().ensureReadable(path);
    expect(mockedReadFile).not.toHaveBeenCalled();
    expect(useEditorStore.getState().tabs[0]?.error).toBe("missing");
  });

  it("turns an undisplayable text read into a binary fallback", async () => {
    mockedReadFile.mockRejectedValue(
      new WorkspaceRequestError("二进制，无法在这里显示", 415),
    );
    await useEditorStore.getState().ensureReadable("weird.txt");
    const tab = useEditorStore.getState().tabs[0]!;
    expect(tab.kind).toBe("binary");
    expect(tab.error).toBe("二进制，无法在这里显示");
    expect(tab.errorRetryable).toBe(false);

    mockedReadFile.mockClear();
    await useEditorStore.getState().ensureReadable("weird.txt");
    expect(mockedReadFile).not.toHaveBeenCalled();
  });

  it("does not read preview files as text", async () => {
    await useEditorStore.getState().ensureReadable("shot.png");
    expect(mockedReadFile).not.toHaveBeenCalled();
    expect(useEditorStore.getState().tabs[0]?.kind).toBe("image");
  });

  it("does not write a preview tab on save", async () => {
    useEditorStore.setState({
      tabs: [
        {
          path: "shot.png",
          content: "",
          savedContent: "",
          dirty: false,
          language: "plaintext",
          loading: false,
          error: null,
          errorRetryable: false,
          kind: "image",
          diskRevision: 0,
        },
      ],
      activePath: "shot.png",
    });
    await useEditorStore.getState().save("shot.png");
    expect(mockedWriteFile).not.toHaveBeenCalled();
  });

  it("bumps a preview revision when the file changes on disk", async () => {
    useEditorStore.setState({
      tabs: [
        {
          path: "shot.png",
          content: "",
          savedContent: "",
          dirty: false,
          language: "plaintext",
          loading: false,
          error: null,
          errorRetryable: false,
          kind: "image",
          diskRevision: 0,
        },
      ],
    });
    await useEditorStore.getState().handleWorkspaceChange(["shot.png"], "modified");
    expect(mockedReadFile).not.toHaveBeenCalled();
    expect(useEditorStore.getState().tabs[0]?.diskRevision).toBe(1);
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

function gridGroup(id: string, components: string[]) {
  return {
    api: { id, location: { type: "grid" as const } },
    panels: components.map((component, index) => ({
      id: `${component}-${index}`,
      api: { component, setActive: vi.fn() },
    })),
  };
}

function fakeDockview(
  groups: ReturnType<typeof gridGroup>[],
  activeGroupId: string,
): DockviewApi & { addPanel: ReturnType<typeof vi.fn> } {
  const addPanel = vi.fn();
  return {
    groups,
    activeGroup: groups.find((group) => group.api.id === activeGroupId),
    getPanel: () => undefined,
    addPanel,
    addGroup: vi.fn(() => ({ id: "new" })),
  } as unknown as DockviewApi & { addPanel: ReturnType<typeof vi.fn> };
}

describe("editor panel placement", () => {
  beforeEach(() => {
    mockedReadFile.mockResolvedValue("body");
  });

  it("opens a file beside the agent group instead of inside it", async () => {
    const api = fakeDockview([gridGroup("agent", ["agent"])], "agent");
    useEditorStore.getState().setDockviewApi(api);
    await useEditorStore.getState().openFile("src/a.ts");
    expect(api.addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        component: "editor",
        position: { referenceGroup: "agent", direction: "right" },
      }),
    );
    useEditorStore.getState().setDockviewApi(null);
  });

  it("reuses an editor group that is not the agent group", async () => {
    const api = fakeDockview(
      [gridGroup("agent", ["agent"]), gridGroup("editors", ["editor"])],
      "agent",
    );
    useEditorStore.getState().setDockviewApi(api);
    await useEditorStore.getState().openFile("src/b.ts");
    expect(api.addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        position: { referenceGroup: "editors" },
      }),
    );
    useEditorStore.getState().setDockviewApi(null);
  });
});
