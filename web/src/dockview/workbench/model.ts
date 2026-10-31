/**
 * Group and panel classification.
 *
 * `location.type === "grid"` is not "the visible main center". A popout
 * leaves the original group on the main grid with `isVisible === false`
 * and size 0. That group is an anchor, not a host for a new panel.
 */

export type GroupRole =
  | "main-center"
  | "popout-center"
  | "popout-anchor"
  | "edge";

export type PanelWindow = "main" | { dockId: string };

export interface GroupFacts {
  id: string;
  locationType: string | undefined;
  isVisible: boolean;
  /** Missing dimensions mean the caller did not measure the group. */
  width: number | undefined;
  height: number | undefined;
  popoutUrl?: string;
  /**
   * The tab the group shows. This survives the group losing keyboard focus,
   * unlike the panel API's `isActive` (which also requires the group to be
   * active), so a sidebar marker can stay on the session the agent area shows.
   */
  activePanel?: {
    id: string;
    component: string | undefined;
    params: Record<string, unknown>;
  };
  panels: { id: string; component: string | undefined }[];
}

export interface PanelWhere {
  panelId: string;
  kind: string;
  zone: "center" | "edge";
  window: PanelWindow;
  groupId: string;
  role: GroupRole;
}

export function groupRole(facts: GroupFacts): GroupRole | null {
  if (facts.locationType === "edge") return "edge";
  if (facts.locationType === "popout") return "popout-center";
  if (facts.locationType === "grid") {
    return facts.isVisible ? "main-center" : "popout-anchor";
  }
  return null;
}

/** A center group that can show a panel. Anchors and zero-size groups cannot. */
export function isUsableCenter(facts: GroupFacts): boolean {
  const role = groupRole(facts);
  if (role !== "main-center" && role !== "popout-center") return false;
  if (facts.width == null && facts.height == null) return true;
  return (facts.width ?? 0) > 0 || (facts.height ?? 0) > 0;
}
