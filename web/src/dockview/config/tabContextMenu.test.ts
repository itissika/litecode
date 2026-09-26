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
  isMaximized: () => boolean;
  exitMaximized: () => void;
  maximize: () => void;
  setTitle: () => void;
  close: () => void;
}

function makePanel(component: string, tabComponent?: string): IDockviewPanel {
  const api: FakePanelApi = {
    component,
    tabComponent,
    title: `${component} title`,
    isMaximized: () => false,
    exitMaximized: () => {},
    maximize: () => {},
    setTitle: () => {},
    close: () => {},
  };
  return { api } as unknown as IDockviewPanel;
}

function makeParams(
  panel: IDockviewPanel,
  allPanels: IDockviewPanel[],
): GetTabContextMenuItemsParams {
  return {
    panel,
    group: { panels: [panel] } as unknown as DockviewGroupPanel,
    api: {
      panels: allPanels,
      addPopoutGroup: () => Promise.resolve(true),
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
    expect(labels(items)).toEqual([
      "Popout Window",
      "separator",
      "Maximize",
      "separator",
      "Rename",
    ]);
  });

  it("keeps Close hidden for non-terminal edge panels even when terminals exist", () => {
    const filetree = makePanel("filetree", "edge");
    const terminal = makePanel("terminal", "edge");
    const items = buildTabContextMenuItems(
      makeParams(filetree, [filetree, terminal]),
    );

    expect(items).not.toContain("close");
    expect(labels(items)).toEqual([
      "Popout Window",
      "separator",
      "Maximize",
      "separator",
      "Rename",
    ]);
  });

  it("keeps the default close items for non-edge (editor) tabs", () => {
    const editor = makePanel("editor");
    const items = buildTabContextMenuItems(makeParams(editor, [editor]));

    expect(items).toEqual(["close", "closeOthers", "closeAll"]);
  });
});
