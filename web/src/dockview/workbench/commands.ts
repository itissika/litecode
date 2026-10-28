import type { DockviewApi, DockviewGroupPanel } from "dockview-react";

import { dockIdFromPopoutUrl, popoutPageUrl } from "../config/popoutUrl";
import { lastUserDockId } from "./activity";
import { dockview } from "./host";
import { isLayoutRestoring } from "./restoreGate";
import { getWindows } from "./windows";
import { kindForComponent } from "./kinds";
import { groupRole, isUsableCenter } from "./model";
import { placementFor, type PanelPlace } from "./placement";
import { popoutScreenBox, type PopoutGroupRect, type PopoutHostBox } from "./popoutBox";
import { groupId, normalizeGroup, readGroup, type GroupLike } from "./readGroup";

export interface OpenPanelRequest {
  id: string;
  component: string;
  title: string;
  tabComponent?: string;
  params?: Record<string, unknown>;
  /** Hint from a header action. Adopted only when that group is a usable main document group. */
  preferredGroupId?: string;
}

interface PanelApiLike {
  component?: string;
  isVisible?: boolean;
  location?: { type?: string; getWindow?: () => Window | null };
  group?: GroupLike;
  setActive: () => void;
  setTitle?: (title: string) => void;
  moveTo?: (options: { group?: DockviewGroupPanel; position?: string }) => void;
  close?: () => void;
}

interface PanelLike {
  api: unknown;
}

function panelApi(panel: PanelLike): PanelApiLike {
  return panel.api as PanelApiLike;
}

function onAnchor(panel: PanelLike): boolean {
  const api = panelApi(panel);
  const group = api.group;
  if (group) {
    const facts = readGroup(group);
    const role = groupRole(facts);
    if (role === "popout-anchor") return true;
    return (role === "main-center" || role === "popout-center") && !isUsableCenter(facts);
  }
  return api.location?.type === "grid" && api.isVisible === false;
}
function hosting(panel: PanelLike): "main" | "popout" | "no" {
  const api = panelApi(panel);
  const group = api.group;
  if (group) {
    const facts = readGroup(group);
    const role = groupRole(facts);
    if (!isUsableCenter(facts)) return "no";
    if (role === "popout-center") return "popout";
    if (role === "main-center") return "main";
    return "no";
  }
  if (api.location?.type === "popout" && api.isVisible !== false) return "popout";
  if (api.location?.type === "grid" && api.isVisible !== false) return "main";
  if (api.isVisible === true) return "main";
  return "no";
}

function focusPopout(panel: PanelLike): void {
  const api = panelApi(panel);
  const location = api.group?.api.location ?? api.location;
  if (location?.type !== "popout") return;
  location.getWindow?.()?.focus();
}

function panelCount(group: GroupLike | null | undefined): number {
  return group?.panels?.length ?? 0;
}

/**
 * The panel is the last tab of a popout window.
 *
 * Moving it empties the popout group, and Dockview then removes that
 * popout's reference group from the main grid when it has no panels
 * (`doRemoveGroup`, popout branch) before it opens the panel on the
 * destination. When the destination is that same empty group (an empty
 * main center, or the anchor `ensureMainCenterGroup` just revealed), the
 * panel is opened into a disposed group and disappears.
 */
function leavesPopoutEmpty(panel: PanelLike): boolean {
  const group = panelApi(panel).group;
  if (!group) return false;
  return group.api.location?.type === "popout" && panelCount(group) <= 1;
}

type ReturnApi = Pick<DockviewApi, "addGroup"> &
  Partial<Pick<DockviewApi, "getGroup" | "removeGroup">>;

/** Remove an empty main-grid group the move left behind, if Dockview did not. */
function dropEmptyLeftover(live: ReturnApi, targetId: string, keepId: string): void {
  if (!targetId || targetId === keepId) return;
  if (typeof live.getGroup !== "function" || typeof live.removeGroup !== "function") return;
  const stale = live.getGroup(targetId) as unknown as GroupLike | undefined;
  if (!stale || panelCount(stale) > 0) return;
  if (stale.api.location?.type !== "grid") return;
  live.removeGroup(stale as unknown as DockviewGroupPanel);
}

/**
 * Bring the last tab of a popout home onto an empty main-grid spot.
 * A fresh group is added beside `target` first, so the cascade that removes
 * the popout's empty reference group cannot take the destination with it.
 * The emptied `target` is then removed so the new group fills the center.
 */
function rehomeOnFreshGroup(live: ReturnApi, panel: PanelLike, target: GroupLike): boolean {
  const api = panelApi(panel);
  if (!api.moveTo) return false;
  const targetId = groupId(target);
  let fresh: GroupLike | null = null;
  if (targetId) {
    try {
      fresh = live.addGroup({ referenceGroup: targetId, direction: "right" }) as unknown as GroupLike;
    } catch {
      fresh = null;
    }
  }
  if (!fresh) {
    try {
      fresh = live.addGroup() as unknown as GroupLike;
    } catch {
      return false;
    }
  }
  const host = normalizeGroup(fresh);
  if (!host) return false;
  api.moveTo({ group: host });
  dropEmptyLeftover(live, targetId, groupId(host as unknown as GroupLike));
  return true;
}

function moveToPlace(live: ReturnApi, panel: PanelLike, place: PanelPlace): void {
  const api = panelApi(panel);
  if (!api.moveTo) return;
  const target = place.group as unknown as GroupLike;
  if (leavesPopoutEmpty(panel) && panelCount(target) === 0) {
    if (rehomeOnFreshGroup(live, panel, target)) return;
  }
  if (place.position.direction) {
    api.moveTo({ group: place.group, position: place.position.direction });
    return;
  }
  api.moveTo({ group: place.group });
}

/**
 * Open or focus a panel.
 *
 * A panel already on a usable surface is activated in place. A popout is
 * focused rather than copied back. A panel sitting on an anchor is moved
 * onto the main center first. A missing panel is added to the window the
 * user last used: a popout window hosts it as a new tab, otherwise it goes
 * to the main center.
 */
export function openPanel(request: OpenPanelRequest, api?: DockviewApi | null): void {
  const live = api ?? dockview();
  if (!live) return;
  const spec = kindForComponent(request.component);
  const tabComponent = request.tabComponent ?? spec?.tabComponent;
  const existing = live.getPanel(request.id);
  if (existing) {
    if (request.title) panelApi(existing).setTitle?.(request.title);
    const where = hosting(existing);
    if (where === "popout") {
      existing.api.setActive();
      focusPopout(existing);
      return;
    }
    if (where === "main" && !onAnchor(existing)) {
      existing.api.setActive();
      return;
    }
    if (onAnchor(existing)) {
      const place = placementFor(request.component, live, request.preferredGroupId);
      if (place) moveToPlace(live, existing, place);
    }
    existing.api.setActive();
    return;
  }
  // Edges recover via addPanel directly; business opens go through here and
  // must not race fromJSON (a panel added mid-restore is wiped).
  if (isLayoutRestoring()) return;
  const place = placementFor(
    request.component,
    live,
    request.preferredGroupId,
    lastUserDockId(),
  );
  if (!place || !tabComponent) return;
  live.addPanel({
    id: request.id,
    component: request.component,
    title: request.title,
    tabComponent,
    params: request.params,
    position: place.position,
  });
}

/** Focus a panel that already exists. Returns false when it does not. */
export function revealPanel(panelId: string): boolean {
  const live = dockview();
  const existing = live?.getPanel(panelId);
  if (!existing || !live) return false;
  openPanel({
    id: panelId,
    component: existing.api.component ?? "editor",
    title: existing.api.title ?? panelId,
    tabComponent: existing.api.tabComponent,
  });
  return true;
}

export function closePanel(panelId: string): void {
  dockview()?.getPanel(panelId)?.api.close();
}

interface PopoutSource {
  getWindow?: () => (PopoutHostBox & { closed?: boolean }) | null;
  group?: { element?: { getBoundingClientRect?: () => PopoutGroupRect } };
}

function sourceOf(panel: PanelLike): PopoutSource {
  return panel.api as PopoutSource;
}

/**
 * Pop a center panel into its own window. Edge panels are refused.
 * `at` is the pointer release in screen coordinates. Without it, the new
 * window sits on the group inside the window that currently holds the tab.
 */
export function popoutPanel(
  panelId: string,
  at?: { screenX: number; screenY: number },
): void {
  const live = dockview();
  const panel = live?.getPanel(panelId);
  if (!live || !panel) return;
  const spec = kindForComponent(panel.api.component);
  if (spec && !spec.canPopout) return;
  if (!spec && panel.api.location?.type === "edge") return;
  const source = sourceOf(panel);
  const host = source.getWindow?.();
  const position = popoutScreenBox(
    host && !host.closed ? host : null,
    source.group?.element?.getBoundingClientRect?.() ?? null,
    at,
  );
  void live
    .addPopoutGroup(panel, {
      popoutUrl: popoutPageUrl(),
      ...(position ? { position } : {}),
    })
    .catch(() => {});
}

/** Bring a panel back to the main-center group its kind belongs in. */
export function movePanelToMain(panelId: string): void {
  const live = dockview();
  const panel = live?.getPanel(panelId);
  if (!live || !panel) return;
  const place = placementFor(panel.api.component ?? "editor", live);
  if (!place) return;
  moveToPlace(live, panel, place);
  panelApi(panel).setActive?.();
}

interface ReturnDropEvent {
  getData: () => { panelId?: string | null } | null | undefined;
  group?: unknown;
  defaultPrevented?: boolean;
  preventDefault: () => void;
}

/**
 * A native drop of a popout's last tab onto an empty main-grid group hits
 * the same Dockview cascade as `movePanelToMain`. Take the drop over and
 * rehome the tab on a fresh group. Returns true when the drop was handled.
 */
export function routePopoutReturnDrop(event: ReturnDropEvent, api?: DockviewApi | null): boolean {
  const live = api ?? dockview();
  if (!live || event.defaultPrevented) return false;
  const panelId = event.getData()?.panelId;
  const target = event.group as GroupLike | undefined;
  if (!panelId || !target) return false;
  if (target.api.location?.type !== "grid" || panelCount(target) > 0) return false;
  const panel = live.getPanel(panelId);
  if (!panel || !leavesPopoutEmpty(panel)) return false;
  event.preventDefault();
  if (!rehomeOnFreshGroup(live, panel, target)) return false;
  panelApi(panel).setActive?.();
  return true;
}

/**
 * Move a panel into a popout window.
 * `dockId` is the id in that window's `/popout.html?dock=` URL.
 */
export function joinPopoutWindow(panelId: string, dockId: string): void {
  const live = dockview();
  const panel = live?.getPanel(panelId);
  if (!live || !panel || !dockIdFromPopoutUrl(popoutPageUrl(dockId))) return;
  const group = getWindows().find((entry) => entry.dockId === dockId)?.group;
  if (!group) return;
  panelApi(panel).moveTo?.({ group: group as unknown as DockviewGroupPanel });
}
