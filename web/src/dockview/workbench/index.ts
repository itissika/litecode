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
 * Features open, reveal, move, and pop panels through this directory (commands.ts, edges.ts).
 * They do not receive Dockview's add, move, or popout methods.
 *
 * A panel component's own props.api may set its title, close itself, and
 * listen for visibility. Cross-panel actions go through this module.
 */

// Only re-exports that have a consumer outside workbench/ (WP-159 / U3).
// Other modules import workbench submodules directly.
export { groupRole } from "./model";
