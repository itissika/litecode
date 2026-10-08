import { describe, expect, it } from "vitest";
import type {
  DockviewApi,
  DockviewGroupPanel,
  IDockviewPanel,
  TabDragEvent,
} from "dockview-react";

import { bindTabDrag, isMainCenterHit } from "./bindTabDrag";

interface Harness {
  onPanel: (event: TabDragEvent) => void;
  onShow: (event: {
    getData: () => { panelId?: string | null; groupId?: string | null } | undefined;
    group?: { api: { location: { type: string } } };
    preventDefault: () => void;
  }) => void;
  onDrop: () => void;
  popouts: IDockviewPanel[];
  moves: DockviewGroupPanel[];
}

const draggedPanels = new Map<string, IDockviewPanel>();

function panel(type: string, getWindow: () => Window = () => window): IDockviewPanel {
  const item = {
    id: `panel-${draggedPanels.size}`,
    api: {
      component: type === "edge" ? "terminal" : "editor",
      location: { type },
      getWindow,
      moveTo: () => {},
      setActive: () => {},
    },
  } as unknown as IDockviewPanel;
  draggedPanels.set(item.id, item);
  return item;
}

function harness(options?: {
  sourceType?: string;
  popoutWindow?: Window;
  popoutGroup?: DockviewGroupPanel;
  gridGroups?: DockviewGroupPanel[];
}): Harness {
  let onPanel: (event: TabDragEvent) => void = () => {};
  let onShow: Harness["onShow"] = () => {};
  let onDrop: () => void = () => {};
  const popouts: IDockviewPanel[] = [];
  const moves: DockviewGroupPanel[] = [];
  const sourceType = options?.sourceType ?? "grid";
  const api = {
    onWillShowOverlay: (cb: Harness["onShow"]) => {
      onShow = cb;
      return { dispose() {} };
    },
    onWillDragPanel: (cb: (event: TabDragEvent) => void) => {
      onPanel = cb;
      return { dispose() {} };
    },
    onWillDragGroup: () => {
      throw new Error("blank tab-bar drag is not a popout");
    },
    onDidDrop: (cb: () => void) => {
      onDrop = cb;
      return { dispose() {} };
    },
    onDidMovePanel: (cb: () => void) => {
      onDrop = cb;
      return { dispose() {} };
    },
    getPanel: (id: string) =>
      draggedPanels.get(id) ?? {
        api: { location: { type: sourceType }, component: "editor" },
      },
    getGroup: () => ({ api: { location: { type: sourceType } } }),
    getPopouts: () =>
      options?.popoutWindow && options.popoutGroup
        ? [{ id: "pop-1", window: options.popoutWindow, group: options.popoutGroup }]
        : [],
    groups: [
      ...(options?.gridGroups ?? []),
      ...(options?.popoutGroup ? [options.popoutGroup] : []),
    ],
    addGroup: () => ({ id: "fresh" }) as unknown as DockviewGroupPanel,
    addPopoutGroup: (item: IDockviewPanel) => {
      popouts.push(item);
      return Promise.resolve(true);
    },
  } as unknown as DockviewApi;
  bindTabDrag(api);
  return {
    get onPanel() {
      return onPanel;
    },
    get onShow() {
      return onShow;
    },
    get onDrop() {
      return onDrop;
    },
    popouts,
    moves,
  };
}

function dragEvent(type: string, init?: MouseEventInit): DragEvent {
  return new MouseEvent(type, init) as DragEvent;
}

function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function settleDrag(): Promise<void> {
  await flush();
  await flush();
}

function outsidePoint(): { screenX: number; screenY: number } {
  return {
    screenX: window.screenX + window.outerWidth + 40,
    screenY: window.screenY + 20,
  };
}

function popoutWindow(): Window {
  return {
    closed: false,
    screenX: window.screenX + window.outerWidth + 200,
    screenY: window.screenY,
    outerWidth: 400,
    outerHeight: 300,
  } as unknown as Window;
}

describe("isMainCenterHit", () => {
  it("accepts the empty center and refuses an edge rail", () => {
    const dock = document.createElement("div");
    dock.className = "dv-dockview";
    const edge = document.createElement("div");
    edge.className = "dv-groupview dv-groupview-edge";
    const watermark = document.createElement("div");
    watermark.className = "dv-watermark";
    dock.append(edge, watermark);
    expect(isMainCenterHit(watermark)).toBe(true);
    expect(isMainCenterHit(edge)).toBe(false);
    expect(isMainCenterHit(document.createElement("div"))).toBe(false);
  });
});

describe("bindTabDrag", () => {
  it("hides the drop mask when a center tab hovers an edge rail", () => {
    const drag = harness();
    let prevented = false;
    drag.onShow({
      getData: () => ({ panelId: "editor" }),
      group: { api: { location: { type: "edge" } } },
      preventDefault: () => {
        prevented = true;
      },
    });
    expect(prevented).toBe(true);
  });

  it("leaves the mask up for a center tab on the center grid and on a root split", () => {
    const drag = harness();
    let prevented = false;
    const prevent = () => {
      prevented = true;
    };
    drag.onShow({
      getData: () => ({ panelId: "editor" }),
      group: { api: { location: { type: "grid" } } },
      preventDefault: prevent,
    });
    drag.onShow({
      getData: () => ({ panelId: "editor" }),
      preventDefault: prevent,
    });
    expect(prevented).toBe(false);
  });

  it("pops a grid tab when the drag ends outside every window", async () => {
    const drag = harness();
    const editor = panel("grid");
    drag.onPanel({ nativeEvent: dragEvent("dragstart"), panel: editor });
    window.dispatchEvent(dragEvent("dragend", outsidePoint()));
    await flush();
    expect(drag.popouts).toEqual([]);
    await flush();
    expect(drag.popouts).toEqual([editor]);
  });

  it("pops a tab that already lives in a popout when released on the desktop", async () => {
    const drag = harness();
    const editor = panel("popout");
    drag.onPanel({ nativeEvent: dragEvent("dragstart"), panel: editor });
    window.dispatchEvent(dragEvent("dragend", outsidePoint()));
    await settleDrag();
    expect(drag.popouts).toEqual([editor]);
  });

  it("joins an existing popout when the release misses its drop target", async () => {
    const win = popoutWindow();
    const group = {
      api: {
        isVisible: true,
        location: { type: "popout", getWindow: () => win },
      },
    } as unknown as DockviewGroupPanel;
    const drag = harness({ popoutWindow: win, popoutGroup: group });
    const editor = panel("grid");
    editor.api.moveTo = (options) => {
      if (options.group) drag.moves.push(options.group);
    };
    drag.onPanel({ nativeEvent: dragEvent("dragstart"), panel: editor });
    window.dispatchEvent(
      dragEvent("dragend", {
        screenX: win.screenX + 20,
        screenY: win.screenY + 20,
      }),
    );
    await settleDrag();
    expect(drag.popouts).toEqual([]);
    expect(drag.moves).toEqual([group]);
  });

  it("brings a popout tab home when released on an empty main center", async () => {
    const home = {
      api: { location: { type: "grid" }, isVisible: true, id: "center" },
    } as unknown as DockviewGroupPanel;
    const drag = harness({ gridGroups: [home] });
    const editor = panel("popout", () => ({ closed: false } as Window));
    editor.api.moveTo = (options) => {
      if (options.group) drag.moves.push(options.group);
    };
    const dock = document.createElement("div");
    dock.className = "dv-dockview";
    const watermark = document.createElement("div");
    watermark.className = "dv-watermark";
    dock.append(watermark);
    document.body.append(dock);
    const previous = document.elementFromPoint?.bind(document);
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: () => watermark,
    });
    drag.onPanel({ nativeEvent: dragEvent("dragstart"), panel: editor });
    window.dispatchEvent(
      dragEvent("dragend", {
        screenX: window.screenX + 30,
        screenY: window.screenY + 30,
      }),
    );
    await settleDrag();
    if (previous) {
      Object.defineProperty(document, "elementFromPoint", {
        configurable: true,
        value: previous,
      });
    }
    dock.remove();
    expect(drag.popouts).toEqual([]);
    expect(drag.moves).toEqual([home]);
  });

  it("leaves a release inside the main window on the grid", async () => {
    const drag = harness();
    drag.onPanel({
      nativeEvent: dragEvent("dragstart"),
      panel: panel("grid"),
    });
    window.dispatchEvent(
      dragEvent("dragend", {
        screenX: window.screenX + 10,
        screenY: window.screenY + 10,
      }),
    );
    await settleDrag();
    expect(drag.popouts).toEqual([]);
  });

  it("does not pop an edge tab", async () => {
    const drag = harness();
    drag.onPanel({
      nativeEvent: dragEvent("dragstart"),
      panel: panel("edge"),
    });
    window.dispatchEvent(dragEvent("dragend", outsidePoint()));
    await settleDrag();
    expect(drag.popouts).toEqual([]);
  });

  it("leaves a tab dockview already accepted on drop", async () => {
    const drag = harness();
    drag.onPanel({
      nativeEvent: dragEvent("dragstart"),
      panel: panel("grid"),
    });
    drag.onDrop();
    window.dispatchEvent(dragEvent("dragend", outsidePoint()));
    await settleDrag();
    expect(drag.popouts).toEqual([]);
  });

  it("pops a pointer drag only after release, and ignores a cancel", async () => {
    const drag = harness();
    const editor = panel("grid");
    drag.onPanel({
      nativeEvent: new PointerEvent("pointermove"),
      panel: editor,
    });
    window.dispatchEvent(new PointerEvent("pointercancel"));
    await settleDrag();
    expect(drag.popouts).toEqual([]);

    drag.onPanel({
      nativeEvent: new PointerEvent("pointermove"),
      panel: editor,
    });
    window.dispatchEvent(new PointerEvent("pointerup", outsidePoint()));
    await settleDrag();
    expect(drag.popouts).toEqual([editor]);
  });
});
