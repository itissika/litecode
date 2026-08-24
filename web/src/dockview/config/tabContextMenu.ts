import type {
  BuiltInContextMenuItem,
  DockviewApi,
  GetTabContextMenuItemsParams,
  ReactContextMenuItemConfig,
} from "dockview-react";

type TabMenuItem = BuiltInContextMenuItem | ReactContextMenuItemConfig;

/**
 * Build the right-click tab context menu.
 *
 * Edge tabs (Explorer / Search / Source Control / Sessions / Terminal) are
 * persistent workspace panels: Popout / Maximize / Rename, but no close
 * actions — with one exception. Each terminal tab owns a live backend PTY
 * process, so once more than one terminal panel exists an explicit Close is
 * offered. Closing removes the panel; TerminalPanel's unmount cleanup sends
 * `terminal/close`, which kills the backend process (whole process tree).
 * The last remaining terminal keeps no Close item (VS Code behavior), so the
 * workspace always retains one terminal. Panels are counted across the whole
 * layout (`api.panels`), not per group, so the item stays available even if
 * terminal tabs are spread over several groups.
 */
export function buildTabContextMenuItems(
  params: GetTabContextMenuItemsParams,
): TabMenuItem[] {
  const { panel, api } = params;

  if (panel.api.tabComponent === "edge") {
    const items: TabMenuItem[] = [
      { label: "Popout Window", action: () => api.addPopoutGroup(panel).catch(() => {}) },
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

    if (panel.api.component === "terminal" && countTerminalPanels(api) > 1) {
      items.push("separator", "close");
    }
    return items;
  }

  return ["close", "closeOthers", "closeAll"];
}

function countTerminalPanels(api: DockviewApi): number {
  return api.panels.filter((p) => p.api.component === "terminal").length;
}
