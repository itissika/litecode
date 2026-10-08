import { afterEach, describe, expect, it, vi } from "vitest";

import { bindDockview } from "../../dockview/workbench/host";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { followCitation, openKnowledgeGraphPanel, revealKnowledgeNode } from "./panel";
import { useEditorStore } from "../../stores/editorStore";

afterEach(() => {
  bindDockview(null);
});

function gridGroup(id: string) {
  return { api: { id, location: { type: "grid" as const }, isVisible: true } };
}

describe("openKnowledgeGraphPanel", () => {
  it("activates a hidden graph panel instead of adding another", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    bindDockview({
      getPanel: vi.fn(() => ({ api: { setActive, isVisible: false, component: "knowledgeGraph" } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("activates an already visible graph panel", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    bindDockview({
      getPanel: vi.fn(() => ({ api: { setActive, isVisible: true, component: "knowledgeGraph" } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("joins the only main group when the active group is an edge rail", () => {
    const addPanel = vi.fn();
    const addGroup = vi.fn();
    bindDockview({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup,
      activeGroup: { api: { id: "edge", location: { type: "edge" } } },
      groups: [
        { api: { location: { type: "edge" }, id: "edge" } },
        { api: { location: { type: "grid" }, id: "g1", isVisible: true } },
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
        position: { referenceGroup: "g1" },
      }),
    );
  });

  it("starts a grid group when the center is empty", () => {
    const addPanel = vi.fn();
    const addGroup = vi.fn(() => ({ id: "fresh" }));
    bindDockview({
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
    bindDockview({
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
    bindDockview({
      activePanel: { id: "knowledge-graph", api: { component: "knowledgeGraph" } },
      addPanel,
      getPanel: vi.fn(),
      groups: [gridGroup("graph")],
      activeGroup: gridGroup("graph"),
    } as never);
    followCitation({ kind: "node", key: "seq" });
    expect(addPanel).not.toHaveBeenCalled();
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
  });

  it("asks the editor to open a workspace file", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt, tabs: [] } as never);
    const addPanel = vi.fn();
    bindDockview({
      activePanel: { id: "agent-1", api: { component: "agent" } },
      activeGroup: gridGroup("src"),
      groups: [gridGroup("src"), gridGroup("editors")],
      getPanel: () => undefined,
      addPanel,
    } as never);
    followCitation({ kind: "file", path: "src/a.rs", line: 4 });
    expect(openFileAt).toHaveBeenCalledWith("src/a.rs", 4);
    expect(openFile).not.toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("activates an external preview instead of reading the path from the workspace", () => {
    const openFile = vi.fn(async () => {});
    useEditorStore.setState({
      openFile,
      tabs: [{ path: "external:C:/outside/a.ts", external: true }],
    } as never);
    const setActive = vi.fn();
    const addPanel = vi.fn();
    bindDockview({
      getPanel: (id: string) =>
        id === "external:C:/outside/a.ts"
          ? { api: { setActive, component: "editor", isVisible: true } }
          : undefined,
      addPanel,
      groups: [gridGroup("src")],
      activeGroup: gridGroup("src"),
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
    bindDockview({
      getPanel: () => undefined,
      addPanel,
      groups: [gridGroup("src")],
      activeGroup: gridGroup("src"),
    } as never);
    followCitation({ kind: "file", path: "C:/outside/a.ts" });
    expect(addPanel).not.toHaveBeenCalled();
    expect(openFile).not.toHaveBeenCalled();
  });
});
