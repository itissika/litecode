import type { DockviewApi } from "dockview-react";

/**
 * The live Dockview instance. Features never receive this object.
 * `startWorkbench` is the production binder; tests bind a stand-in.
 */

let current: DockviewApi | null = null;
const attached = new Set<() => void>();

export function bindDockview(api: DockviewApi | null): void {
  current = api;
  if (!api) return;
  for (const listener of attached) listener();
}

export function dockview(): DockviewApi | null {
  return current;
}

export function onDockviewAttached(listener: () => void): () => void {
  attached.add(listener);
  if (current) listener();
  return () => {
    attached.delete(listener);
  };
}
