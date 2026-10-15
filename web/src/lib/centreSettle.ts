/**
 * "Centre settled" signal for startup ordering. Leaf module (no imports) so
 * the connection/session stores can report into it without an import cycle;
 * `centreChat` decides which path applies.
 *
 * Resolves once per page load when:
 * - empty centre: the `session/new` it sent has replied (ok or error);
 * - restored centre: a `session/list` reply/push arrived, or a panel's
 *   `session/subscribe` completed;
 * - the transport failed (disconnected / reconnecting) first.
 * Non-critical startup requests (engines, settings summary, file tree root)
 * wait on it so they do not compete with the first chat. No timeout.
 */

let settled = false;
/** Centre already had panels (restored layout): settle on list or attach. */
let restoredCentre = false;
/** Latched: a session/list reply or push arrived (any time this page load). */
let listSettled = false;
/** Latched: a panel's session/subscribe completed. */
let attached = false;
let resolveSettled!: () => void;
let settledPromise = fresh();

function fresh(): Promise<void> {
  return new Promise<void>((resolve) => {
    resolveSettled = resolve;
  });
}

export function whenCentreSettled(): Promise<void> {
  return settledPromise;
}

export function settleCentre(): void {
  if (settled) return;
  settled = true;
  resolveSettled();
}

function maybeSettleRestored(): void {
  if (restoredCentre && (listSettled || attached)) settleCentre();
}

/** centreChat found panels in the restored centre (no session/new). */
export function noteRestoredCentre(): void {
  restoredCentre = true;
  maybeSettleRestored();
}

/** A `session/list` reply (success or failure) or push arrived. */
export function noteSessionListSettled(): void {
  listSettled = true;
  maybeSettleRestored();
}

/** A panel's `session/subscribe` completed. */
export function noteRootAttached(): void {
  attached = true;
  maybeSettleRestored();
}

/** The socket dropped or failed before the centre settled: release waiters. */
export function noteTransportFailed(): void {
  settleCentre();
}

export function resetCentreSettleForTests(): void {
  settled = false;
  restoredCentre = false;
  listSettled = false;
  attached = false;
  settledPromise = fresh();
}
