export interface PanelRemoved {
  id: string;
  component: string | undefined;
}

const listeners = new Set<(event: PanelRemoved) => void>();

export function onPanelRemoved(
  listener: (event: PanelRemoved) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function emitPanelRemoved(event: PanelRemoved): void {
  for (const listener of listeners) listener(event);
}
