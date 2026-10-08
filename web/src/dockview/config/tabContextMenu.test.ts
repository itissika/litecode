import { describe, expect, it } from "vitest";
import type {
  DockviewApi,
  DockviewGroupPanel,
  GetTabContextMenuItemsParams,
  IDockviewPanel,
} from "dockview-react";

import { buildTabContextMenuItems } from "./tabContextMenu";

interface FakePanelApi {
  component: string;
  tabComponent?: string;
  title?: string;
  location?: { type: string };
  isMaximized: () => boolean;
  exitMaximized: () => void;
  maximize: () => void;
  setTitle: () => void;
  close: () => void;
  moveTo: (options: { group?: unknown }) => void;
}

function makePanel(
  component: string,
  tabComponent?: string,
  locationType?: string,
): IDockviewPanel {
  const api: FakePanelApi = {
    component,
    tabComponent,
    title: `${component} title`,
    location: locationType ? { type: locationType } : undefined,
    isMaximized: () => false,
    exitMaximized: () => {},
    maximize: () => {},
    setTitle: () => {},
    close: () => {},
    moveTo: () => {},
  };
  return { id: component, api } as unknown as IDockviewPanel;
}

function makeParams(
  panel: IDockviewPanel,
  allPanels: IDockviewPanel[],
  api: Partial<DockviewApi> = {},
): GetTabContextMenuItemsParams {
  return {
    panel,
    group: { panels: [panel] } as unknown as DockviewGroupPanel,
    api: {
      panels: allPanels,
      groups: [],
      ...api,
      getPanel: (id: string) => allPanels.find((item) => item.id === id),
    } as unknown as DockviewApi,
    event: {} as MouseEvent,
  };
}

function labels(
  items: ReturnType<typeof buildTabContextMenuItems>,
): (string | undefined)[] {
  return items.map((item) => (typeof item === "string" ? item : item.label));
}

describe("buildTabContextMenuItems", () => {
  it("keeps Close hidden on a terminal edge tab", () => {
    const terminal1 = makePanel("terminal", "edge");
    const terminal2 = makePanel("terminal", "edge");
    const items = buildTabContextMenuItems(
      makeParams(terminal1, [terminal1, terminal2]),
    );

    expect(items).not.toContain("close");
    expect(labels(items)).toEqual(["Maximize", "separator", "Rename"]);
  });

  it("keeps Close hidden for non-terminal edge panels even when terminals exist", () => {
    const filetree = makePanel("filetree", "edge");
    const terminal = makePanel("terminal", "edge");
    const items = buildTabContextMenuItems(
      makeParams(filetree, [filetree, terminal]),
    );

    expect(items).not.toContain("close");
    expect(labels(items)).toEqual(["Maximize", "separator", "Rename"]);
  });

  it("keeps the default close items for non-edge tabs that are not in the grid", () => {
    const editor = makePanel("editor");
    const items = buildTabContextMenuItems(makeParams(editor, [editor]));

    expect(items).toEqual(["close", "closeOthers", "closeAll"]);
  });

  it("offers popout for a grid tab and records a dock id on the popout page", () => {
    const editor = makePanel("editor", "editor", "grid");
    const browser = makePanel("browser", "browser", "grid");
    const calls: Array<{ panel: IDockviewPanel | DockviewGroupPanel; url?: string }> = [];
    const addPopoutGroup: DockviewApi["addPopoutGroup"] = (panel, options) => {
      calls.push({ panel, url: options?.popoutUrl });
      return Promise.resolve(true);
    };

    const editorItems = buildTabContextMenuItems(
      makeParams(editor, [editor], { addPopoutGroup }),
    );
    const browserItems = buildTabContextMenuItems(
      makeParams(browser, [browser], { addPopoutGroup }),
    );
    expect(labels(editorItems)).toEqual([
      "Popout Window",
      "separator",
      "close",
      "closeOthers",
      "closeAll",
    ]);
    expect(labels(browserItems)[0]).toBe("Popout Window");

    const popout = editorItems[0];
    if (typeof popout === "string" || !popout.action) {
      throw new Error("expected a popout action");
    }
    popout.action();
    expect(calls).toHaveLength(1);
    expect(calls[0]?.panel).toBe(editor);
    expect(calls[0]?.url).toMatch(
      /^\/popout\.html\?dock=[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
    );
  });

  it("returns a popout tab to a visible main-grid group", () => {
    const editor = makePanel("editor", "editor", "popout");
    const moves: unknown[] = [];
    editor.api.moveTo = (options) => {
      moves.push(options.group);
    };
    const hidden = {
      api: { location: { type: "grid" }, isVisible: false },
    };
    const home = {
      api: { location: { type: "grid" }, isVisible: true, id: "center" },
    };
    const items = buildTabContextMenuItems(
      makeParams(editor, [editor], {
        groups: [hidden, home] as unknown as DockviewApi["groups"],
        addGroup: () => {
          throw new Error("a visible grid group is already there");
        },
      }),
    );

    expect(labels(items)).toEqual([
      "Return to Main Window",
      "separator",
      "close",
      "closeOthers",
      "closeAll",
    ]);

    const back = items[0];
    if (typeof back === "string" || !back.action) {
      throw new Error("expected a return action");
    }
    back.action();
    expect(moves).toEqual([home]);
  });

  it("opens a main-grid group when the center has nothing visible to return to", () => {
    const editor = makePanel("editor", "editor", "popout");
    const moves: unknown[] = [];
    editor.api.moveTo = (options) => {
      moves.push(options.group);
    };
    const created = { api: { id: "fresh" } };
    const items = buildTabContextMenuItems(
      makeParams(editor, [editor], {
        groups: [
          { api: { location: { type: "grid" }, isVisible: false } },
        ] as unknown as DockviewApi["groups"],
        addGroup: () => created as unknown as DockviewGroupPanel,
      }),
    );

    const back = items[0];
    if (typeof back === "string" || !back.action) {
      throw new Error("expected a return action");
    }
    back.action();
    expect(moves).toEqual([created]);
  });
});
