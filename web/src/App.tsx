import { useEffect } from "react";

import { AppShellDockview } from "./dockview/AppShellDockview";
import { ToastHost } from "./components/ToastHost";
import { installDebugConsole } from "./lib/debugTrace";
import { useConnectionStore } from "./stores/connectionStore";

export default function App() {
  const init = useConnectionStore((s) => s.init);
  const destroy = useConnectionStore((s) => s.destroy);

  useEffect(() => {
    installDebugConsole();
    init();
    return () => destroy();
  }, [init, destroy]);

  return (
    <>
      <AppShellDockview />
      <ToastHost />
    </>
  );
}
