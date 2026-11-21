import { createContext, useContext, type ReactNode } from "react";

/**
 * Whether the panel that owns this transcript is on screen.
 *
 * Dockview keeps every open panel mounted (`defaultRenderer: "always"`)
 * and hides the inactive ones with `display: none`. Their React tree keeps
 * running: store flushes, timers, markdown parses. A streaming transcript is
 * the expensive one, so the streaming clock reads this flag and shows the
 * authoritative text instead of animating while the panel is not visible.
 *
 * The default is `true`: anything rendered outside a panel (toasts, knowledge
 * documents, tests) behaves exactly as before.
 */
const PanelVisibleContext = createContext(true);

export function PanelVisibilityProvider({
  active,
  children,
}: {
  active: boolean;
  children: ReactNode;
}) {
  return (
    <PanelVisibleContext.Provider value={active}>
      {children}
    </PanelVisibleContext.Provider>
  );
}

export function usePanelVisible(): boolean {
  return useContext(PanelVisibleContext);
}
