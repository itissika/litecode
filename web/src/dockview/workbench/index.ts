/**
 * LiteCode panel manager.
 *
 * Dockview constraints this layer relies on:
 *
 * - Popping a grid group out leaves that group on the main grid with
 *   location "grid" and isVisible false (size 0). It is a return anchor,
 *   not a host. The live group has location "popout". The window registry
 *   lists those live groups by dock id.
 * - setActive selects a panel inside its current group. It does not show
 *   an anchor and does not move a panel to the main window. Activating a
 *   popout group focuses that window.
 * - setVisible(true) on an anchor restores its cached size and redistributes
 *   the grid. addGroup() is only for a center that has no anchor left.
 * - Closing a popout moves its panels onto the anchor and shows that anchor
 *   when the anchor still exists. Otherwise the group is reattached on the
 *   main grid.
 *
 * Features open, reveal, move, and pop panels through the functions below.
 * They do not receive Dockview's add, move, or popout methods.
 *
 * A panel component's own props.api may set its title, close itself, and
 * listen for visibility. Cross-panel actions go through this module.
 */

export type {
  CenterPlacement,
  EdgeRail,
  PanelKindSpec,
  Zone,
} from "./kinds";
export { PANEL_KINDS, edgeKinds, kindForComponent } from "./kinds";
export type { GroupFacts, GroupRole, PanelWhere, PanelWindow } from "./model";
export { groupRole, isUsableCenter } from "./model";
export type { CenterGroupQuery, PopoutWindowInfo } from "./queries";
export {
  activePanelComponent,
  activePanelId,
  centerGroups,
  hasPanel,
  isInMainWindow,
  mainCenterHasPanel,
  panelVisible,
  popoutWindows,
  roleOfGroup,
  whereIs,
} from "./queries";
export { bindDockview, onDockviewAttached } from "./host";
export type { PanelRemoved } from "./events";
export { onPanelRemoved } from "./events";
export type { OpenPanelRequest } from "./commands";
export {
  closePanel,
  joinPopoutWindow,
  movePanelToMain,
  openPanel,
  popoutPanel,
  revealPanel,
} from "./commands";
export {
  ensureDefaultEdges,
  ensureEdgePanel,
  recoverDefaultLayout,
  revealEdgePanel,
} from "./edges";
export type { LayoutStore } from "./lifecycle";
export { LAYOUT_SCHEMA_VERSION, layoutStore, startWorkbench } from "./lifecycle";
export { ensureMainCenterGroup, placementFor, resetPlacementForTests } from "./placement";
