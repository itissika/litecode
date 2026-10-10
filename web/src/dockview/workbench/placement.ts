import type { DockviewApi, DockviewGroupPanel } from "dockview-react";

import { dockview } from "./host";
import { kindForComponent, type CenterPlacement } from "./kinds";
import { groupRole, isUsableCenter } from "./model";
import { groupId, normalizeGroup, readGroup, type GroupLike } from "./readGroup";

const recentMainGroups: string[] = [];

export function noteActiveGroup(group: GroupLike | undefined): void {
  if (!group) return;
  const facts = readGroup(group);
  if (groupRole(facts) !== "main-center" || !facts.id) return;
  const next = recentMainGroups.filter((id) => id !== facts.id);
  next.push(facts.id);
  recentMainGroups.splice(0, recentMainGroups.length, ...next.slice(-12));
}

export function resetPlacementForTests(): void {
  recentMainGroups.splice(0, recentMainGroups.length);
}

function liveApi(api?: DockviewApi | null): DockviewApi | null {
  return api ?? dockview();
}

function groupsOf(api: DockviewApi): GroupLike[] {
  return api.groups as unknown as GroupLike[];
}

function usableMainGroups(api: DockviewApi): GroupLike[] {
  return groupsOf(api).filter((group) => {
    const facts = readGroup(group);
    return groupRole(facts) === "main-center" && isUsableCenter(facts);
  });
}

/** Main-center groups including ones still at 0×0 before the first layout pass. */
function anyMainCenterGroups(api: DockviewApi): GroupLike[] {
  return groupsOf(api).filter((group) => groupRole(readGroup(group)) === "main-center");
}

function anchors(api: DockviewApi): GroupLike[] {
  return groupsOf(api).filter(
    (group) => groupRole(readGroup(group)) === "popout-anchor",
  );
}

function hasAgent(group: GroupLike): boolean {
  return (group.panels ?? []).some((panel) => {
    const spec = kindForComponent(panel.api.component);
    return spec?.placement === "agent";
  });
}

function hasDocument(group: GroupLike): boolean {
  return (group.panels ?? []).some((panel) => {
    const spec = kindForComponent(panel.api.component);
    return spec?.placement === "document";
  });
}

function pickMain(
  candidates: GroupLike[],
  active: GroupLike | undefined,
): DockviewGroupPanel | null {
  if (candidates.length === 0) return null;
  if (active && candidates.some((group) => groupId(group) === groupId(active))) {
    return active as unknown as DockviewGroupPanel;
  }
  return candidates[0] as unknown as DockviewGroupPanel;
}

/**
 * A main-center group to host a new panel.
 *
 * Prefer an already-measured center, then any main-center (even 0×0 right
 * after fromJSON), then a revealed popout anchor. Never `addGroup` beside a
 * zero-size center/anchor: that races the layout pass and leaves NEW in a
 * side strip instead of the real center. `addGroup()` only when the grid
 * has no center host left (watermark-only).
 */
export function ensureMainCenterGroup(api?: DockviewApi | null): DockviewGroupPanel | null {
  const live = liveApi(api);
  if (!live) return null;
  const active = live.activeGroup as unknown as GroupLike | undefined;

  const usable = pickMain(usableMainGroups(live), active);
  if (usable) return usable;

  const pending = pickMain(anyMainCenterGroups(live), active);
  if (pending) return pending;

  const anchor = anchors(live)[0];
  if (anchor?.api.setVisible) {
    anchor.api.setVisible(true);
    // Dockview restores cached size on show; size may still read 0 until
    // paint. Hosting on the anchor avoids splitting NEW off-center.
    return anchor as unknown as DockviewGroupPanel;
  }
  return normalizeGroup(live.addGroup() as unknown as GroupLike);
}

export interface PanelPlace {
  group: DockviewGroupPanel;
  position: { referenceGroup: string; direction?: "right" };
}

function placeOn(group: GroupLike, direction?: "right"): PanelPlace {
  const id = groupId(group);
  return {
    group: group as unknown as DockviewGroupPanel,
    position: direction ? { referenceGroup: id, direction } : { referenceGroup: id },
  };
}

function besidePlace(api: DockviewApi): PanelPlace | null {
  const usable = usableMainGroups(api);
  const active = api.activeGroup as unknown as GroupLike | undefined;
  const source =
    active && usable.some((group) => groupId(group) === groupId(active))
      ? active
      : undefined;
  const others = usable.filter((group) => groupId(group) !== (source ? groupId(source) : ""));
  if (others.length === 0) {
    if (source) return placeOn(source, "right");
    const created = ensureMainCenterGroup(api);
    if (!created) return null;
    return placeOn(created as unknown as GroupLike);
  }
  if (others.length === 1) return placeOn(others[0]!);
  const otherIds = new Set(others.map((group) => groupId(group)));
  let chosen = others[others.length - 1]!;
  for (let index = recentMainGroups.length - 1; index >= 0; index -= 1) {
    const id = recentMainGroups[index];
    if (id && otherIds.has(id) && id !== (source ? groupId(source) : undefined)) {
      const match = others.find((group) => groupId(group) === id);
      if (match) chosen = match;
      break;
    }
  }
  return placeOn(chosen);
}

function documentPlace(api: DockviewApi): PanelPlace | null {
  const usable = usableMainGroups(api);
  const active = api.activeGroup as unknown as GroupLike | undefined;
  if (
    active &&
    usable.some((group) => groupId(group) === groupId(active)) &&
    !hasAgent(active)
  ) {
    return placeOn(active);
  }
  const editorGroup = usable.find((group) => !hasAgent(group) && hasDocument(group));
  if (editorGroup) return placeOn(editorGroup);
  const agentGroup = usable.find((group) => hasAgent(group));
  if (agentGroup) return placeOn(agentGroup, "right");
  const created = ensureMainCenterGroup(api);
  if (!created) return null;
  return placeOn(created as unknown as GroupLike);
}

function agentPlace(api: DockviewApi): PanelPlace | null {
  // Include zero-size main-center: restored agent groups can read 0×0 until paint.
  const agentGroup = anyMainCenterGroups(api).find((group) => hasAgent(group));
  if (agentGroup) return placeOn(agentGroup);
  const created = ensureMainCenterGroup(api);
  if (!created) return null;
  return placeOn(created as unknown as GroupLike);
}

export function placementFor(
  component: string,
  api?: DockviewApi | null,
  preferredGroupId?: string,
): PanelPlace | null {
  const live = liveApi(api);
  if (!live) return null;
  const spec = kindForComponent(component);
  const placement: CenterPlacement = spec?.placement ?? "document";
  if (preferredGroupId && placement === "document") {
    const preferred = usableMainGroups(live).find(
      (group) => groupId(group) === preferredGroupId && !hasAgent(group),
    );
    if (preferred) return placeOn(preferred);
  }
  if (placement === "agent") return agentPlace(live);
  if (placement === "beside") return besidePlace(live);
  return documentPlace(live);
}
