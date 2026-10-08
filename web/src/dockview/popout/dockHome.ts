import type { DockviewApi, DockviewGroupPanel, IDockviewPanel } from "dockview-react";

import { isMainGrid } from "./location";

/**
 * A popped-out tab comes back onto a visible main-grid group.
 * Popping a whole group leaves a hidden reference group behind; landing
 * there would hide the tab. When the center grid has nothing visible,
 * open a group first.
 */
export function dockPanelToMain(api: DockviewApi, panel: IDockviewPanel): void {
  const home = visibleMainGridGroup(api) ?? api.addGroup();
  panel.api.moveTo({ group: home });
}

function visibleMainGridGroup(api: DockviewApi): DockviewGroupPanel | undefined {
  return api.groups.find(
    (group) => isMainGrid(group.api.location.type) && group.api.isVisible,
  );
}
