/**
 * One declaration per dock panel kind.
 *
 * Drag rules, the tab menu, and where a new panel lands are read from this
 * list. Features pass a component name; they do not re-state the rules.
 */

export type Zone = "center" | "edge";

export type EdgeRail = "left" | "right" | "bottom";

/** Where a new center panel sits relative to the groups already open. */
export type CenterPlacement = "agent" | "document" | "beside";

export interface PanelKindSpec {
  component: string;
  tabComponent: string;
  zone: Zone;
  rail?: EdgeRail;
  canPopout: boolean;
  singletonId?: string;
  placement?: CenterPlacement;
  title?: string;
}

export const PANEL_KINDS: readonly PanelKindSpec[] = [
  {
    component: "filetree",
    tabComponent: "edge",
    zone: "edge",
    rail: "left",
    canPopout: false,
    singletonId: "filetree",
    title: "Explorer",
  },
  {
    component: "search",
    tabComponent: "edge",
    zone: "edge",
    rail: "left",
    canPopout: false,
    singletonId: "workspace-search",
    title: "Search",
  },
  {
    component: "git",
    tabComponent: "edge",
    zone: "edge",
    rail: "left",
    canPopout: false,
    singletonId: "workspace-git",
    title: "Source Control",
  },
  {
    component: "knowledge",
    tabComponent: "edge",
    zone: "edge",
    rail: "left",
    canPopout: false,
    singletonId: "workspace-knowledge",
    title: "Knowledge",
  },
  {
    component: "sessions",
    tabComponent: "edge",
    zone: "edge",
    rail: "right",
    canPopout: false,
    singletonId: "sessions",
    title: "Sessions",
  },
  {
    component: "terminal",
    tabComponent: "edge",
    zone: "edge",
    rail: "bottom",
    canPopout: false,
    singletonId: "workspace-terminal",
    title: "Terminal",
  },
  {
    component: "editor",
    tabComponent: "editor",
    zone: "center",
    canPopout: true,
    placement: "document",
  },
  {
    component: "agent",
    tabComponent: "agent",
    zone: "center",
    canPopout: true,
    placement: "agent",
  },
  {
    component: "subagent",
    tabComponent: "agent",
    zone: "center",
    canPopout: true,
    placement: "agent",
  },
  {
    component: "knowledgeGraph",
    tabComponent: "knowledgeGraph",
    zone: "center",
    canPopout: true,
    singletonId: "knowledge-graph",
    placement: "beside",
    title: "Knowledge Graph",
  },
  {
    component: "browser",
    tabComponent: "browser",
    zone: "center",
    canPopout: true,
    placement: "document",
  },
  {
    component: "about",
    tabComponent: "default",
    zone: "center",
    canPopout: false,
    placement: "document",
    title: "About",
  },
];

export function kindForComponent(
  component: string | undefined,
): PanelKindSpec | undefined {
  if (!component) return undefined;
  return PANEL_KINDS.find((kind) => kind.component === component);
}

export function edgeKinds(): PanelKindSpec[] {
  return PANEL_KINDS.filter((kind) => kind.zone === "edge");
}
