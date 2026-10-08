import { afterEach, describe, expect, it, vi } from "vitest";

import { setDockviewApi } from "../../stores/connectionStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { followCitation, openKnowledgeGraphPanel, placeGridPanel, revealKnowledgeNode } from "./panel";
import { useEditorStore } from "../../stores/editorStore";

afterEach(() => {
  setDockviewApi(null);
});

describe("openKnowledgeGraphPanel", () => {
  it("activates a hidden graph panel instead of adding another", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => ({ api: { setActive, isVisible: false } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("leaves an already visible graph panel where it is", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => ({ api: { setActive, isVisible: true } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).not.toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("splits a new group beside the current grid when none is open", () => {
    const addPanel = vi.fn();
    const addGroup = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup,
      activeGroup: { api: { id: "edge", location: { type: "edge" } } },
      groups: [
        { api: { location: { type: "edge" }, id: "edge" } },
        { api: { location: { type: "grid" }, id: "g1" } },
      ],
    } as never);
    openKnowledgeGraphPanel();
    expect(addGroup).not.toHaveBeenCalled();
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "knowledge-graph",
        component: "knowledgeGraph",
        title: "Knowledge Graph",
        tabComponent: "knowledgeGraph",
        position: { referenceGroup: "g1", direction: "right" },
      }),
    );
  });

  it("starts a grid group when the center is empty", () => {
    const addPanel = vi.fn();
    const addGroup = vi.fn(() => ({ id: "fresh" }));
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup,
      groups: [],
    } as never);
    openKnowledgeGraphPanel();
    expect(addGroup).toHaveBeenCalled();
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        position: { referenceGroup: "fresh" },
      }),
    );
  });
});

describe("revealKnowledgeNode", () => {
  it("focuses the node and opens the graph", () => {
    useKnowledgeStore.setState({
      byId: new Map([["seq", { id: "seq" }]]),
    } as never);
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(() => ({ id: "fresh" })),
      groups: [],
    } as never);
    revealKnowledgeNode("seq");
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(addPanel).toHaveBeenCalled();
  });
});

function gridGroup(id: string) {
  return { api: { id, location: { type: "grid" as const } } };
}

describe("placeGridPanel", () => {
  it("splits a group only when the center has no other layout", () => {
    const addPanel = vi.fn();
    placeGridPanel(
      {
        getPanel: () => undefined,
        addPanel,
        onDidActiveGroupChange: () => ({ dispose() {} }),
        activeGroup: gridGroup("src"),
        groups: [gridGroup("src")],
      } as never,
      { id: "src/a.rs", component: "editor", title: "a.rs" },
    );
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        position: { referenceGroup: "src", direction: "right" },
      }),
    );
  });

  it("drops a new panel into the only other grid group", () => {
    const addPanel = vi.fn();
    placeGridPanel(
      {
        getPanel: () => undefined,
        addPanel,
        onDidActiveGroupChange: () => ({ dispose() {} }),
        activeGroup: gridGroup("src"),
        groups: [gridGroup("src"), gridGroup("editors")],
      } as never,
      { id: "src/a.rs", component: "editor", title: "a.rs" },
    );
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        position: { referenceGroup: "editors" },
      }),
    );
  });

  it("uses the most recently active other group when several exist", () => {
    const addPanel = vi.fn();
    const changed: { current: (group: ReturnType<typeof gridGroup>) => void } = {
      current: () => undefined,
    };
    const api = {
      getPanel: () => undefined,
      addPanel,
      onDidActiveGroupChange: (listener: (group: ReturnType<typeof gridGroup>) => void) => {
        changed.current = listener;
        return { dispose() {} };
      },
      activeGroup: gridGroup("src"),
      groups: [gridGroup("src"), gridGroup("fresh"), gridGroup("stale")],
    };
    placeGridPanel(api as never, { id: "one", component: "editor", title: "one" });
    expect(addPanel).toHaveBeenLastCalledWith(
      expect.objectContaining({ position: { referenceGroup: "stale" } }),
    );
    changed.current(gridGroup("fresh"));
    changed.current(gridGroup("src"));
    placeGridPanel(api as never, { id: "two", component: "editor", title: "two" });
    expect(addPanel).toHaveBeenLastCalledWith(
      expect.objectContaining({ position: { referenceGroup: "fresh" } }),
    );
  });

  it("activates a panel that is already open", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    placeGridPanel(
      {
        getPanel: () => ({ api: { setActive } }),
        addPanel,
        groups: [],
      } as never,
      { id: "src/a.rs", component: "editor", title: "a.rs" },
    );
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });
});

describe("followCitation", () => {
  const originalOpenFile = useEditorStore.getState().openFile;
  const originalOpenFileAt = useEditorStore.getState().openFileAt;
  const originalTabs = useEditorStore.getState().tabs;

  afterEach(() => {
    useEditorStore.setState({
      openFile: originalOpenFile,
      openFileAt: originalOpenFileAt,
      tabs: originalTabs,
    });
  });

  it("focuses the card and does not add a panel while the graph is showing", () => {
    useKnowledgeStore.setState({
      byId: new Map([["seq", { id: "seq", key: "seq" }]]),
      byKey: new Map([["seq", { id: "seq", key: "seq" }]]),
    } as never);
    const addPanel = vi.fn();
    setDockviewApi({
      activePanel: { id: "knowledge-graph", api: { component: "knowledgeGraph" } },
      addPanel,
      getPanel: vi.fn(),
      groups: [gridGroup("graph")],
      activeGroup: gridGroup("graph"),
      onDidActiveGroupChange: () => ({ dispose() {} }),
    } as never);
    followCitation({ kind: "node", key: "seq" });
    expect(addPanel).not.toHaveBeenCalled();
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
  });

  it("asks the editor to open a workspace file in the other grid group", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt, tabs: [] } as never);
    const addPanel = vi.fn();
    setDockviewApi({
      activePanel: { id: "agent-1", api: { component: "agent" } },
      activeGroup: gridGroup("src"),
      groups: [gridGroup("src"), gridGroup("editors")],
      getPanel: () => undefined,
      addPanel,
      onDidActiveGroupChange: () => ({ dispose() {} }),
    } as never);
    followCitation({ kind: "file", path: "src/a.rs", line: 4 });
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "src/a.rs",
        component: "editor",
        position: { referenceGroup: "editors" },
      }),
    );
    expect(openFileAt).toHaveBeenCalledWith("src/a.rs", 4);
    expect(openFile).not.toHaveBeenCalled();
  });

  it("activates an external preview instead of reading the path from the workspace", () => {
    const openFile = vi.fn(async () => {});
    useEditorStore.setState({
      openFile,
      tabs: [{ path: "external:C:/outside/a.ts", external: true }],
    } as never);
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: (id: string) =>
        id === "external:C:/outside/a.ts" ? { api: { setActive } } : undefined,
      addPanel,
      groups: [gridGroup("src")],
      activeGroup: gridGroup("src"),
      onDidActiveGroupChange: () => ({ dispose() {} }),
    } as never);
    followCitation({ kind: "file", path: "C:/outside/a.ts" });
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
    expect(openFile).not.toHaveBeenCalled();
  });

  it("does not open an external path that has no preview", () => {
    const openFile = vi.fn(async () => {});
    useEditorStore.setState({ openFile, tabs: [] } as never);
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: () => undefined,
      addPanel,
      groups: [gridGroup("src")],
      activeGroup: gridGroup("src"),
      onDidActiveGroupChange: () => ({ dispose() {} }),
    } as never);
    followCitation({ kind: "file", path: "C:/outside/a.ts" });
    expect(addPanel).not.toHaveBeenCalled();
    expect(openFile).not.toHaveBeenCalled();
  });
});
