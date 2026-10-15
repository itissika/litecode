import type { RemoteProgressEvent } from "./bridge-contract";

export type { RemoteProgressEvent };

/** Minimal shape of a ComponentProgressEvent used for Open Remote mirroring. */
export type ComponentProgressLike = {
  id: string;
  phase: string;
  received?: number;
  total?: number;
  message: string;
};

/**
 * Map a component download fraction into an Open Remote bar sub-range [from, to].
 * Missing/zero total → the low end of the range (0% of the slice).
 */
export function remoteDownloadRatio(
  range: readonly [number, number],
  received: number | undefined,
  total: number | undefined,
): number {
  const [from, to] = range;
  const frac = total && total > 0 ? Math.min(1, Math.max(0, (received ?? 0) / total)) : 0;
  return from + (to - from) * frac;
}

/**
 * Translate a component progress event into a remote-bar download event.
 * Returns null for the wrong id or error phases (errors must surface via ensure throw).
 */
export function mapComponentProgressToRemote(
  event: ComponentProgressLike,
  id: string,
  range: readonly [number, number],
): RemoteProgressEvent | null {
  if (event.id !== id || event.phase === "error") return null;
  return {
    stage: "download",
    ratio: remoteDownloadRatio(range, event.received, event.total),
    message: event.message,
  };
}

export type EnsureComponentForRemoteDeps<T> = {
  id: string;
  range: readonly [number, number];
  ensure: (id: string) => Promise<T>;
  subscribe: (listener: (event: ComponentProgressLike) => void) => void;
  unsubscribe: (listener: (event: ComponentProgressLike) => void) => void;
  emit: (event: RemoteProgressEvent) => void;
};

/**
 * Ensure a component while mirroring non-error download progress into the Open Remote bar.
 * Failures from `ensure` propagate unchanged; the listener is always removed.
 */
export async function ensureComponentForRemote<T>(
  deps: EnsureComponentForRemoteDeps<T>,
): Promise<T> {
  const listener = (event: ComponentProgressLike): void => {
    const mapped = mapComponentProgressToRemote(event, deps.id, deps.range);
    if (mapped) deps.emit(mapped);
  };
  deps.subscribe(listener);
  try {
    return await deps.ensure(deps.id);
  } finally {
    deps.unsubscribe(listener);
  }
}
