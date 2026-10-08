import type { DockviewGroupPanel } from "dockview-react";

import type { GroupFacts } from "./model";

interface LocationLike {
  type?: string;
  popoutUrl?: string;
  getWindow?: () => Window | null;
}

interface GroupApiLike {
  id?: string;
  location?: LocationLike;
  isVisible?: boolean;
  width?: number;
  height?: number;
  setVisible?: (visible: boolean) => void;
  expand?: () => void;
}

export interface GroupLike {
  id?: string;
  api: GroupApiLike;
  panels?: { id: string; api: { component?: string } }[];
}

export function groupId(group: GroupLike): string {
  return group.api.id ?? group.id ?? "";
}

export function readGroup(group: GroupLike): GroupFacts {
  const location = group.api.location;
  return {
    id: groupId(group),
    locationType: location?.type,
    isVisible: group.api.isVisible !== false,
    width: group.api.width,
    height: group.api.height,
    popoutUrl: location?.popoutUrl,
    panels: (group.panels ?? []).map((panel) => ({
      id: panel.id,
      component: panel.api.component,
    })),
  };
}

/** `addGroup()` in tests sometimes returns `{ id }` without an `api`. */
export function normalizeGroup(
  group: GroupLike | null | undefined,
): DockviewGroupPanel | null {
  if (!group) return null;
  if (group.api?.id || group.api?.location) {
    return group as unknown as DockviewGroupPanel;
  }
  const id = group.id ?? "";
  return {
    id,
    api: { id, location: { type: "grid" }, isVisible: true },
    panels: [],
  } as unknown as DockviewGroupPanel;
}
