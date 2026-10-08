import { dockIdFromPopoutUrl } from "../config/popoutUrl";

/**
 * Electron window id carried in the popout URL dockview stores on the group.
 * Whether a group is the main center, a popout, or an edge rail is a group
 * role. Callers that need that answer use the panel manager.
 */
export function dockIdFromLocation(location: {
  type: string;
  popoutUrl?: string;
} | undefined): string | null {
  if (location?.type !== "popout") return null;
  return dockIdFromPopoutUrl(location.popoutUrl);
}
