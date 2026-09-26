import type {
  BuiltInContextMenuItem,
  GetTabContextMenuItemsParams,
  ReactContextMenuItemConfig,
} from "dockview-react";

type TabMenuItem = BuiltInContextMenuItem | ReactContextMenuItemConfig;

/**
 * Build the right-click tab context menu.
 *
 * Edge tabs (Explorer / Search / Source Control / Sessions / Terminal) are
 * persistent workspace panels: Popout / Maximize / Rename, and no close.
 * Extra shells live in the terminal panel's own list; closing one there
 * kills that pty. The dockview terminal tab itself stays.
 */
export function buildTabContextMenuItems(
  params: GetTabContextMenuItemsParams,
): TabMenuItem[] {
  const { panel, api } = params;

  if (panel.api.tabComponent === "edge") {
    return [
      {
        label: "Popout Window",
        action: () => api.addPopoutGroup(panel).catch(() => {}),
      },
      "separator",
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

  return ["close", "closeOthers", "closeAll"];
}
