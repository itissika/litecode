/**
 * Pure helpers for managed-remote handle bookkeeping (D8).
 * Kept out of main.ts so node:test can cover them without Electron.
 */

export function clearManagedRemoteIfSession<T extends { session: unknown }>(
  current: T | null,
  session: unknown,
): T | null {
  return current?.session === session ? null : current;
}

/** Assign managed remote only after a successful attach (D8). */
export function managedRemoteAfterSuccessfulConnect<T>(
  previous: T | null,
  next: T,
): T {
  void previous;
  return next;
}
