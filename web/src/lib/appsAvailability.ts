/** Apps are a local-workspace feature. A remote workspace has no local app
 * process to talk to, so the rail is hidden rather than showing broken state. */
export function appsAvailable(): boolean {
  return window.litecode?.getSessionMode?.() !== "remote";
}
