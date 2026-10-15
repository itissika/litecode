/**
 * WP-169 boot ordering (S1/S1b/S2), kept free of Electron so it can be tested.
 *
 * 1. The backend (local sidecar / remote health wait) starts and, in parallel,
 *    a skeleton window is created and shown.
 * 2. Only the workbench `loadURL` waits for the backend (READY + attach check
 *    + /health), and it goes into the SAME window.
 * 3. Boot-time `recordRecent` runs once, after that load succeeds.
 * 4. Backend failure: close the skeleton and rethrow the original error.
 * 5. User closes the skeleton: abort the backend start through the AbortSignal
 *    (the sidecar child is killed immediately, no wait for READY), stop a
 *    backend that is already up, and reject with BootAbortedError so the
 *    caller shows no error dialog.
 */

export type SkeletonWindow = {
  isDestroyed(): boolean;
  once(event: "closed", listener: () => void): unknown;
};

export class BootAbortedError extends Error {
  constructor() {
    super("Boot aborted: window closed during startup");
    this.name = "BootAbortedError";
  }
}

export function isBootAborted(error: unknown): error is BootAbortedError {
  return error instanceof BootAbortedError;
}

export type BootSequenceDeps<W extends SkeletonWindow, H> = {
  /**
   * Start the backend. Called before the skeleton is awaited (parallel).
   * `signal` aborts when the user closes the skeleton; the start must then
   * kill what it spawned and reject.
   */
  startBackend(signal: AbortSignal): Promise<H>;
  /** Create + show the skeleton window (and register it, e.g. instance registry). */
  createSkeleton(): Promise<W>;
  /** Backend handle is available (e.g. publish it for quit-time cleanup). */
  onBackend?(handle: H): void;
  /** Post-start checks (attach verify, /health, watchers) → workbench URL. */
  prepare(handle: H): Promise<string>;
  /** Navigate the skeleton window to the workbench. */
  loadWorkbench(win: W, url: string): Promise<void>;
  /** Stop a backend handle (abort path). */
  stopBackend(handle: H): Promise<void>;
  /** Close the skeleton on a failed boot (must not count as a user close). */
  closeSkeleton(win: W): void;
  /** Boot-time recents write (S2). Only called after loadWorkbench succeeds. */
  recordRecent?(): void;
  /** Receives the abort cleanup promise as soon as the user closes the window. */
  onAbort?(cleanup: Promise<void>): void;
};

export async function runBootSequence<W extends SkeletonWindow, H>(
  deps: BootSequenceDeps<W, H>,
): Promise<W> {
  const controller = new AbortController();
  const backend = deps.startBackend(controller.signal);
  // Observed below; avoid an unhandled rejection while the skeleton is created.
  backend.catch(() => undefined);

  let win: W;
  try {
    win = await deps.createSkeleton();
  } catch (error) {
    await backend.then((h) => deps.stopBackend(h)).catch(() => undefined);
    throw error;
  }

  let failing = false;
  let aborted = false;
  let rejectClosed: (error: Error) => void = () => undefined;
  const closed = new Promise<never>((_resolve, reject) => {
    rejectClosed = reject;
  });
  closed.catch(() => undefined);

  const onClosed = () => {
    if (failing) return;
    aborted = true;
    const abortError = new BootAbortedError();
    // A start still in flight kills its child and rejects right away; a
    // backend that is already up is stopped.
    controller.abort(abortError);
    const cleanup = backend.then(
      (h) => deps.stopBackend(h).catch(() => undefined),
      () => undefined,
    );
    deps.onAbort?.(cleanup);
    rejectClosed(abortError);
  };
  if (win.isDestroyed()) onClosed();
  else win.once("closed", onClosed);

  const untilClosed = <T>(work: Promise<T>): Promise<T> => Promise.race([work, closed]);

  try {
    const handle = await untilClosed(backend);
    deps.onBackend?.(handle);
    const url = await untilClosed(deps.prepare(handle));
    await untilClosed(deps.loadWorkbench(win, url));
  } catch (error) {
    if (aborted) throw new BootAbortedError();
    failing = true;
    if (!win.isDestroyed()) deps.closeSkeleton(win);
    throw error;
  }
  deps.recordRecent?.();
  return win;
}
