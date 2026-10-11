import { useEffect } from "react";

import { AppShellDockview } from "./dockview/AppShellDockview";
import { ClickShield } from "./components/ClickShield";
import { ToastHost } from "./components/ToastHost";
import { installEmbedInstallWatcher } from "./dockview/panels/settings/engines/embedInstall";
import { installDebugConsole } from "./lib/debugTrace";
import { useConnectionStore } from "./stores/connectionStore";

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

  return (
    <>
      <AppShellDockview />
      <ToastHost />
      <ClickShield />
    </>
  );
}
