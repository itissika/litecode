import type {
  BuiltInContextMenuItem,
  GetTabContextMenuItemsParams,
  ReactContextMenuItemConfig,
} from "dockview-react";

import { dockPanelToMain } from "../popout/dockHome";
import { isMainGrid, isPopout } from "../popout/location";
import { popoutPageUrl } from "./popoutUrl";

type TabMenuItem = BuiltInContextMenuItem | ReactContextMenuItemConfig;

/**
 * Build the right-click tab context menu.
 *
 * Edge tabs (Explorer / Search / Source Control / Sessions / Terminal) are
 * permanent rails: Maximize / Rename, and no close. Dockview refuses to
 * pop an edge group out, so there is no Popout item.
 * Grid tabs (editor, agent, browser, and the rest of the center) can open
 * in a separate window. The popout URL carries the dock id the desktop host
 * uses for that window. Theme, the tab-bar drag strip, and a layout-size
 * watch attach when dockview reports the window (`onDidAddPopoutGroup`).
 * A tab that is already out gets a way back onto the main grid. Dragging
 * that tab past the main window and releasing uses the same popout call.
 * Extra shells live in the terminal panel's own list; closing one there
 * kills that pty. The dockview terminal tab itself stays.
 */
export function buildTabContextMenuItems(
  params: GetTabContextMenuItemsParams,
): TabMenuItem[] {
  const { panel, api } = params;

  if (panel.api.tabComponent === "edge") {
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
  if (isPopout(location)) {
    return [
      {
        label: "Return to Main Window",
        action: () => dockPanelToMain(api, panel),
      },
      "separator",
      ...items,
    ];
  }
  if (!isMainGrid(location)) return items;
  return [
    {
      label: "Popout Window",
      action: () => {
        void api
          .addPopoutGroup(panel, { popoutUrl: popoutPageUrl() })
          .catch(() => {});
      },
    },
    "separator",
    ...items,
  ];
}
