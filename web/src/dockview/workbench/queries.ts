import type { DockviewApi } from "dockview-react";

import { dockIdFromPopoutUrl } from "../config/popoutUrl";
import { dockview, onDockviewAttached } from "./host";
import { getWindows } from "./windows";
import { kindForComponent } from "./kinds";
import {
  groupRole,
  isUsableCenter,
  type GroupFacts,
  type GroupRole,
  type PanelWhere,
  type PanelWindow,
} from "./model";
import { readGroup, type GroupLike } from "./readGroup";

export interface CenterGroupQuery {
  window?: "main" | "popout";
  usable?: boolean;
}

export interface PopoutWindowInfo {
  dockId: string;
  groupId: string;
}

function apiOr(api?: DockviewApi | null): DockviewApi | null {
  return api ?? dockview();
}

function dockIdOf(group: GroupLike): string | null {
  const location = group.api.location;
  if (location?.type !== "popout") return null;
  return dockIdFromPopoutUrl(location.popoutUrl) ?? null;
}

function windowOf(facts: GroupFacts, group: GroupLike): PanelWindow {
  const role = groupRole(facts);
  if (role !== "popout-center") return "main";
  const dockId = dockIdOf(group);
  return dockId ? { dockId } : "main";
}

export function whereIs(panelId: string, api?: DockviewApi | null): PanelWhere | null {
  const live = apiOr(api);
  if (!live) return null;
  for (const group of live.groups as unknown as GroupLike[]) {
    const facts = readGroup(group);
    const panel = facts.panels.find((item) => item.id === panelId);
    if (!panel) continue;
    const role = groupRole(facts);
    const spec = kindForComponent(panel.component);
    if (!role || !spec) return null;
    return {
      panelId,
      kind: spec.component,
      zone: spec.zone,
      window: windowOf(facts, group),
      groupId: facts.id,
      role,
    };
  }
  const found = live.getPanel(panelId);
  if (!found) return null;
  const group = found.api.group as unknown as GroupLike | undefined;
  if (!group) return null;
  const facts = readGroup(group);
  const role = groupRole(facts);
  const spec = kindForComponent(found.api.component);
  if (!role || !spec) return null;
  return {
    panelId,
    kind: spec.component,
    zone: spec.zone,
    window: windowOf(facts, group),
    groupId: facts.id,
    role,
  };
}

export function centerGroups(
  query: CenterGroupQuery = {},
  api?: DockviewApi | null,
): GroupFacts[] {
  const live = apiOr(api);
  if (!live) return [];
  return (live.groups as unknown as GroupLike[])
    .map((group) => readGroup(group))
    .filter((facts) => {
      const role = groupRole(facts);
      if (query.window === "main") {
        if (role !== "main-center" && role !== "popout-anchor") return false;
      } else if (query.window === "popout") {
        if (role !== "popout-center") return false;
      } else if (role !== "main-center" && role !== "popout-center" && role !== "popout-anchor") {
        return false;
      }
      if (query.usable && !isUsableCenter(facts)) return false;
      return true;
    });
}

export function isInMainWindow(panelId: string): boolean {
  const where = whereIs(panelId);
  return where?.window === "main";
}

/** Popout windows currently registered. The main window is not included. */
export function popoutWindows(): PopoutWindowInfo[] {
  const listed: PopoutWindowInfo[] = [];
  for (const entry of getWindows()) {
    if (!entry.dockId || !entry.groupId) continue;
    listed.push({ dockId: entry.dockId, groupId: entry.groupId });
  }
  return listed;
}

export function hasPanel(panelId: string): boolean {
  return !!dockview()?.getPanel(panelId);
}

export function panelVisible(panelId: string): boolean {
  return dockview()?.getPanel(panelId)?.api.isVisible ?? false;
}

function disposeSub(sub: unknown): void {
  if (
    sub &&
    typeof sub === "object" &&
    "dispose" in sub &&
    typeof sub.dispose === "function"
  ) {
    sub.dispose();
  }
}

/** Recompute when a panel's visibility or the layout changes. */
export function watchPanelVisible(panelId: string, onChange: () => void): () => void {
  let stopVis: (() => void) | undefined;
  let stopLayout: (() => void) | undefined;

  const arm = () => {
    stopVis?.();
    stopLayout?.();
    stopVis = undefined;
    stopLayout = undefined;
    const api = dockview();
    if (!api) return;
    const bindVis = () => {
      if (stopVis) return;
      const panel = api.getPanel(panelId) as
        | { api: { onDidVisibilityChange?: (cb: () => void) => unknown } }
        | undefined;
      const sub = panel?.api.onDidVisibilityChange?.(onChange);
      if (sub) stopVis = () => disposeSub(sub);
    };
    bindVis();
    const layout = api.onDidLayoutChange?.(() => {
      onChange();
      bindVis();
    });
    if (layout) stopLayout = () => disposeSub(layout);
    onChange();
  };

  const off = onDockviewAttached(arm);
  return () => {
    off();
    stopVis?.();
    stopLayout?.();
  };
}

export function activePanelComponent(): string | undefined {
  return dockview()?.activePanel?.api.component;
}

export function activePanelId(): string | undefined {
  return dockview()?.activePanel?.id;
}

/** Dock components that render a session transcript. */
const SESSION_PANEL_COMPONENTS = new Set(["agent", "subagent"]);

/**
 * Sessions shown by the active panel of every live center group.
 *
 * This deliberately reads each group's `activePanel` instead of the panel
 * API's `isActive`: that flag also requires the group to own keyboard focus,
 * so clicking the sidebar (the moment a marker matters most) would clear it.
 * A split center can show several sessions at once — each is reported.
 */
export function activeCenterSessionIds(api?: DockviewApi | null): string[] {
  const live = apiOr(api);
  if (!live) return [];
  const ids = new Set<string>();
  const groups = (live.groups ?? []) as unknown as GroupLike[];
  for (const group of groups) {
    const facts = readGroup(group);
    const role = groupRole(facts);
    if (role !== "main-center" && role !== "popout-center") continue;
    const panel = facts.activePanel;
    if (!panel?.component || !SESSION_PANEL_COMPONENTS.has(panel.component)) {
      continue;
    }
    const sessionId = panel.params.sessionId;
    if (typeof sessionId === "string" && sessionId) ids.add(sessionId);
  }
  return [...ids];
}

/** Subscribe to {@link activeCenterSessionIds}; fires only when it changes. */
export function watchActiveCenterSessionIds(
  onChange: (ids: readonly string[]) => void,
): () => void {
  let stopLayout: (() => void) | undefined;
  let stopResync: (() => void) | undefined;
  let last: readonly string[] = [];

  const sync = () => {
    const next = activeCenterSessionIds();
    if (
      next.length === last.length &&
      next.every((id, index) => id === last[index])
    ) {
      return;
    }
    last = next;
    onChange(next);
  };

  const arm = () => {
    stopLayout?.();
    stopLayout = undefined;
    stopResync?.();
    stopResync = undefined;
    const api = dockview();
    if (!api) {
      sync();
      return;
    }
    const layout = api.onDidLayoutChange?.(sync);
    if (layout) stopLayout = () => disposeSub(layout);
    // The sidebar can mount while a saved layout is still being restored, and
    // that restore's layout change is not replayed to a listener that attached
    // mid-restore. Re-read after the current frame so a cold start does not
    // leave the marker empty until the next user-driven activation.
    const frame = requestAnimationFrame(sync);
    stopResync = () => cancelAnimationFrame(frame);
    sync();
  };

  const off = onDockviewAttached(arm);
  return () => {
    off();
    stopLayout?.();
    stopResync?.();
  };
}

/** True when the main center already hosts any panel (size ignored).
 *
 * Right after fromJSON, groups can read 0x0 until paint. Requiring a usable
 * size here falsely treats restored center panels as missing and spawns NEW
 * into a split via addGroup.
 */
export function mainCenterHasPanel(api?: DockviewApi | null): boolean {
  const live = apiOr(api);
  if (!live) return false;
  return live.panels.some((panel) => {
    const group = panel.api.group as unknown as GroupLike | undefined;
    if (group) {
      return groupRole(readGroup(group)) === "main-center";
    }
    // No group handle: a visible grid panel counts; anchors are not visible.
    return panel.api.location?.type === "grid" && panel.api.isVisible !== false;
  });
}

export function roleOfGroup(group: GroupLike): GroupRole | null {
  return groupRole(readGroup(group));
}
