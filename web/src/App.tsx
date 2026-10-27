import { useEffect } from "react";

import { AppShellDockview } from "./dockview/AppShellDockview";
import { ClickShield } from "./components/ClickShield";
import { ToastHost } from "./components/ToastHost";
import { installEmbedInstallWatcher } from "./dockview/panels/settings/engines/embedInstall";
import { installDebugConsole } from "./lib/debugTrace";
import {
  closeBrowserPanel,
  openAgentBrowserPanel,
} from "./lib/browserPanel";
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

  // The agent can ask the visible workbench to open/close a browser tab. The
  // native page still lives in main; this only manages the dockview panel.
  useEffect(() => {
    const unsubscribeOpen = window.litecode?.onBrowserOpenRequest?.(({ id, url }) => {
      if (!id) return;
      openAgentBrowserPanel(id, typeof url === "string" ? url : "");
    });
    const unsubscribeClose = window.litecode?.onBrowserCloseRequest?.(({ id }) => {
      if (id) closeBrowserPanel(id);
    });
    return () => {
      unsubscribeOpen?.();
      unsubscribeClose?.();
    };
  }, []);

  return (
    <>
      <AppShellDockview />
      <ToastHost />
      <ClickShield />
    </>
  );
}
