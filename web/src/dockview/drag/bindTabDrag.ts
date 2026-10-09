import type { DockviewApi, IDockviewPanel } from "dockview-react";

import { joinPopoutWindow, movePanelToMain, popoutPanel } from "../workbench/commands";
import { bindDockview } from "../workbench/host";
import {
  MAIN_WINDOW_KEY,
  decideRelease,
  rejectsDockTarget,
  tabClass,
  windowAtPoint,
  type ScreenWindow,
  type WindowBox,
} from "./tabDragPolicy";

interface DragPayload {
  panelId?: string | null;
  groupId?: string | null;
}

/** Location of the tab or group that started the drag. */
export function dragSourceLocation(
  data: DragPayload | null | undefined,
  api: DockviewApi,
): string | undefined {
  if (data?.panelId) return api.getPanel(data.panelId)?.api.location.type;
  if (data?.groupId) return api.getGroup(data.groupId)?.api.location.type;
  return undefined;
}

function boxOf(win: Window): WindowBox {
  return {
    screenX: win.screenX,
    screenY: win.screenY,
    outerWidth: win.outerWidth,
    outerHeight: win.outerHeight,
  };
}

function screenWindows(api: DockviewApi): { windows: ScreenWindow[]; byKey: Map<string, Window> } {
  const windows: ScreenWindow[] = [{ key: MAIN_WINDOW_KEY, box: boxOf(window) }];
  const byKey = new Map<string, Window>();
  const seen = new Set<Window>();
  for (const popout of api.getPopouts()) {
    if (popout.window.closed || seen.has(popout.window)) continue;
    seen.add(popout.window);
    windows.push({ key: popout.id, box: boxOf(popout.window) });
    byKey.set(popout.id, popout.window);
  }
  return { windows, byKey };
}

function hostView(event: Event): Window {
  const view = (event as Event & { view?: Window | null }).view;
  if (view && !view.closed) return view;
  return window;
}

function screenPoint(event: Event): { screenX: number; screenY: number } | null {
  if (!("screenX" in event) || !("screenY" in event)) return null;
  const point = event as MouseEvent;
  if (typeof point.screenX !== "number" || typeof point.screenY !== "number") return null;
  return { screenX: point.screenX, screenY: point.screenY };
}

function isHtml5Drag(event: Event): boolean {
  return event.type.startsWith("drag") || "dataTransfer" in event;
}

/** The main window's center, including an empty watermark. Edge rails are not it. */
export function isMainCenterHit(hit: Element | null): boolean {
  if (!hit) return false;
  if (hit.closest(".dv-groupview-edge")) return false;
  return hit.closest(".dv-dockview") !== null;
}

function aimsAtMainCenter(point: { screenX: number; screenY: number }): boolean {
  const x = point.screenX - window.screenX;
  const y = point.screenY - window.screenY;
  if (x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) return false;
  const hit = document.elementFromPoint(x, y);
  return isMainCenterHit(hit);
}

/**
 * One subscription for tab drag.
 * Hover and drop ask the same zone rule, so a refused rail never draws a
 * drop mask. Releasing a center tab outside every LiteCode window opens a
 * popout; releasing over an existing popout joins that window when Dockview
 * did not already accept the drop. The blank tab bar and group chips do not
 * come through `onWillDragPanel`, so they stay inside the dock.
 * The popout waits until the drag session's own cleanup timer has run.
 * Opening the window from `dragend` itself leaves the empty popout page up.
 */
export function bindTabDrag(api: DockviewApi): void {
  bindDockview(api);
  api.onWillShowOverlay((event) => {
    const data = event.getData();
    const sourcePanel = data?.panelId ? api.getPanel(data.panelId) : undefined;
    const source = dragSourceLocation(data, api);
    if (rejectsDockTarget(source, event.group?.api.location.type, sourcePanel?.api.component)) {
      event.preventDefault();
    }
  });

  const arm = (nativeEvent: Event, panel: IDockviewPanel) => {
    if (tabClass(panel.api.location?.type, panel.api.component) !== "center") return;
    const view = hostView(nativeEvent);

    let placed = false;
    let settled = false;
    const markPlaced = () => {
      placed = true;
    };
    const dropSub = api.onDidDrop(markPlaced);
    const moveSub = api.onDidMovePanel(markPlaced);
    const stopWatchingPlacement = () => {
      dropSub.dispose();
      moveSub.dispose();
    };

    const finish = (event: Event) => {
      if (settled) return;
      settled = true;
      view.removeEventListener("dragend", finish, true);
      view.removeEventListener("pointerup", finish);
      view.removeEventListener("pointercancel", finish);
      if (event.type === "pointercancel") {
        stopWatchingPlacement();
        return;
      }
      const point = screenPoint(event);
      view.setTimeout(() => {
        view.setTimeout(() => {
          stopWatchingPlacement();
          if (!point || placed) return;
          const { windows, byKey } = screenWindows(api);
          const key = windowAtPoint(point, windows);
          const decision = decideRelease(
            tabClass(panel.api.location?.type, panel.api.component),
            key,
          );
          if (decision === "popout") {
            popoutPanel(panel.id, point);
            return;
          }
          if (decision === "stay") {
            // An empty center has no drop target, so Dockview never accepts
            // the drop. A release on that center still comes home.
            if (panel.api.getWindow() !== window && aimsAtMainCenter(point)) {
              movePanelToMain(panel.id);
            }
            return;
          }
          if (decision !== "join" || !key) return;
          const target = byKey.get(key);
          if (!target || panel.api.getWindow() === target) return;
          joinPopoutWindow(panel.id, key);
        }, 0);
      }, 0);
    };

    if (isHtml5Drag(nativeEvent)) {
      view.addEventListener("dragend", finish, true);
      return;
    }
    view.addEventListener("pointerup", finish);
    view.addEventListener("pointercancel", finish);
  };

  api.onWillDragPanel((event) => arm(event.nativeEvent, event.panel));
}
