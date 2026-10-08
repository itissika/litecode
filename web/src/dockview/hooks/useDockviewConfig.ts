import { useCallback, useRef } from "react";
import type { DockviewApi, DockviewWillDropEvent } from "dockview-react";

import { buildTabContextMenuItems } from "../config/tabContextMenu";
import { dragSourceLocation } from "../drag/bindTabDrag";
import { rejectsDockTarget } from "../drag/tabDragPolicy";
import { startWorkbench } from "../workbench/lifecycle";

/** Center grid and popout windows are one dock. Edge rails stay on their own. */
export function preventCrossZoneDrop(event: DockviewWillDropEvent, api: DockviewApi) {
  const source = dragSourceLocation(event.getData(), api);
  if (rejectsDockTarget(source, event.group?.api.location.type)) event.preventDefault();
}

export function useDockviewConfig() {
  const apiRef = useRef<DockviewApi | null>(null);

  const onReady = useCallback((event: { api: DockviewApi }) => {
    apiRef.current = event.api;
    startWorkbench(event.api);
  }, []);

  const onWillDrop = useCallback((event: DockviewWillDropEvent) => {
    if (apiRef.current) {
      preventCrossZoneDrop(event, apiRef.current);
    }
  }, []);

  return { apiRef, onReady, onWillDrop, getTabContextMenuItems: buildTabContextMenuItems };
}
