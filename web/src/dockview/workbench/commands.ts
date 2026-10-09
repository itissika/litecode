import type { DockviewApi, DockviewGroupPanel } from "dockview-react";

import { popoutPageUrl } from "../config/popoutUrl";
import { dockview } from "./host";
import { kindForComponent } from "./kinds";
import { groupRole, isUsableCenter } from "./model";
import { placementFor, type PanelPlace } from "./placement";
import { popoutScreenBox, type PopoutGroupRect, type PopoutHostBox } from "./popoutBox";
import { readGroup, type GroupLike } from "./readGroup";

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

function moveToPlace(panel: PanelLike, place: PanelPlace): void {
  const api = panelApi(panel);
  if (!api.moveTo) return;
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
 * onto the main center first. A missing panel is added on the main center,
 * even when a popout window is in front.
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
      if (place) moveToPlace(existing, place);
    }
    existing.api.setActive();
    return;
  }
  const place = placementFor(request.component, live, request.preferredGroupId);
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
  moveToPlace(panel, place);
  panelApi(panel).setActive?.();
}

/**
 * Move a panel into a popout window.
 * `windowKey` is either that window's group id or the dock id in its URL.
 */
export function joinPopoutWindow(panelId: string, windowKey: string): void {
  const live = dockview();
  const panel = live?.getPanel(panelId);
  if (!live || !panel || !live.getPopouts) return;
  const match = live.getPopouts().find((popout) => {
    if (popout.id === windowKey) return true;
    const group = popout.group as unknown as GroupLike;
    const href = popout.window?.location?.href;
    return (
      group.api.location?.popoutUrl?.includes(windowKey) === true ||
      (href ? href.includes(`dock=${windowKey}`) : false)
    );
  });
  const group = match?.group;
  if (!group) return;
  panelApi(panel).moveTo?.({ group });
}
