import type {
  BuiltInContextMenuItem,
  GetTabContextMenuItemsParams,
  ReactContextMenuItemConfig,
} from "dockview-react";

import { bindDockview } from "../workbench/host";
import { kindForComponent } from "../workbench/kinds";
import { movePanelToMain, popoutPanel } from "../workbench/commands";

type TabMenuItem = BuiltInContextMenuItem | ReactContextMenuItemConfig;

/**
 * Build the right-click tab context menu.
 *
 * Edge tabs are permanent rails: Maximize / Rename, and no close.
 * A center kind that can leave the window gets Popout. A tab that is
 * already out gets a way back onto the main center, into the group its
 * kind belongs in. The actions go through the panel manager.
 */
export function buildTabContextMenuItems(
  params: GetTabContextMenuItemsParams,
): TabMenuItem[] {
  const { panel, api } = params;
  bindDockview(api);

  const spec = kindForComponent(panel.api.component);
  if (panel.api.tabComponent === "edge" || spec?.zone === "edge") {
    return [
      panel.api.isMaximized()
        ? { label: "Restore", action: () => panel.api.exitMaximized() }
        : { label: "Maximize", action: () => panel.api.maximize() },
      "separator",
      {
        label: "Rename",
        action: () => {
          const name = prompt("Panel name:", panel.api.title);
          if (name) panel.api.setTitle(name);
        },
      },
    ];
  }

  const items: TabMenuItem[] = ["close", "closeOthers", "closeAll"];
  const location = panel.api.location?.type;
  if (location === "popout") {
    return [
      {
        label: "Return to Main Window",
        action: () => {
        bindDockview(api);
        movePanelToMain(panel.id);
      },
      },
      "separator",
      ...items,
    ];
  }
  const canPopout = spec ? spec.canPopout : location === "grid";
  if (!canPopout || location !== "grid") return items;
  return [
    {
      label: "Popout Window",
      action: () => {
        bindDockview(api);
        popoutPanel(panel.id);
      },
    },
    "separator",
    ...items,
  ];
}
