import { useEffect, useState } from "react";

/** The slice of a dockview panel api this hook needs (kept structural so tests
 *  do not have to build a whole panel). */
export interface PanelVisibilityApi {
  readonly isVisible: boolean;
  onDidVisibilityChange(
    handler: (event: { isVisible: boolean }) => void,
  ): { dispose: () => void };
}

/**
 * Whether this dock panel is on screen right now.
 *
 * Dockview keeps every open panel mounted (`defaultRenderer: "always"`) and
 * hides the inactive ones with `display: none`; their React tree keeps running.
 * This is `isVisible` (hidden vs shown), deliberately NOT `isActive` (focused
 * vs unfocused): a panel in a second split group is unfocused but still read
 * by a human, so its streaming animation must keep running.
 */
export function usePanelVisible(api: PanelVisibilityApi): boolean {
  const [visible, setVisible] = useState(api.isVisible);

  useEffect(() => {
    // The api can report a newer value between render and subscribe.
    setVisible(api.isVisible);
    const subscription = api.onDidVisibilityChange((event) => {
      setVisible(event.isVisible);
    });
    return () => subscription.dispose();
  }, [api]);

  return visible;
}
