import { useEffect } from "react";

import { AppShellDockview } from "./dockview/AppShellDockview";
import { ClickShield } from "./components/ClickShield";
import { ToastHost } from "./components/ToastHost";
import { installEmbedInstallWatcher } from "./dockview/panels/settings/engines/embedInstall";
import { installDebugConsole } from "./lib/debugTrace";
import { useConnectionStore } from "./stores/connectionStore";
import { openKnownSessionPanel } from "./lib/sessionPanelNav";
import { useSessionStore } from "./stores/sessionStore";

export default function App() {
  const init = useConnectionStore((s) => s.init);
  const destroy = useConnectionStore((s) => s.destroy);

  useEffect(() => {
    installDebugConsole();
    init();
    const stopEmbedWatcher = installEmbedInstallWatcher();
    return () => {
      stopEmbedWatcher();
      destroy();
    };
  }, [init, destroy]);

  // Notification click: bring the matching session forward. Unknown ids fail
  // closed to the read-only panel inside openKnownSessionPanel.
  useEffect(() => {
    const unsubscribe = window.litecode?.onTurnNotificationClick?.(({ sessionId }) => {
      if (!sessionId) return;
      openKnownSessionPanel(sessionId, useSessionStore.getState().sessions);
    });
    return () => unsubscribe?.();
  }, []);

  return (
    <>
      <AppShellDockview />
      <ToastHost />
      <ClickShield />
    </>
  );
}
