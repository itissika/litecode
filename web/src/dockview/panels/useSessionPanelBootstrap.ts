import { useEffect, useState } from "react";
import type { DockviewPanelApi } from "dockview-react";

import type { SessionInfo } from "../../api/types";
import { releaseSessionTab } from "../../components/sessionTeardown";
import { useConnectionStore } from "../../stores/connectionStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";

/**
 * Connection lifecycle shared by the session dock panels (writable
 * `AgentPanel` and read-only `SubagentReadOnlyPanel`):
 * - tracks the dockview active state;
 * - (re)subscribes on every transition to "connected", including reconnects,
 *   so a dropped subscription self-heals;
 * - closes the panel with a toast when the session no longer exists (any
 *   other failure waits for the next "connected" transition);
 * - releases the subscription and local projection only on real unmount.
 * Returns the active flag plus the session's list entry and trimmed preview.
 */
export function useSessionPanelBootstrap(
  api: DockviewPanelApi,
  sessionId: string,
): { isActive: boolean; session: SessionInfo | undefined; preview: string } {
  const connState = useConnectionStore((s) => s.state);
  const [isActive, setIsActive] = useState(api.isActive);

  useEffect(() => {
    const d = api.onDidActiveChange((e) => setIsActive(e.isActive));
    return () => d.dispose();
  }, [api]);

  useEffect(() => {
    if (!sessionId || connState !== "connected") return;
    let disposed = false;
    useConnectionStore
      .getState()
      .ensureSubscribe(sessionId)
      .catch((error: unknown) => {
        if (disposed) return;
        const message =
          error instanceof Error ? error.message : "Failed to open session";
        if (/session.*not found/i.test(message)) {
          useToastStore
            .getState()
            .showToast("This session no longer exists", "error");
          api.close();
        }
      });
    return () => {
      disposed = true;
    };
  }, [api, sessionId, connState]);

  useEffect(() => {
    if (!sessionId) return;
    return () => {
      releaseSessionTab(sessionId);
    };
  }, [sessionId]);

  const preview = useSessionStore(
    (s) => s.sessions.find((x) => x.id === sessionId)?.preview?.trim() ?? "",
  );
  const session = useSessionStore((s) =>
    s.sessions.find((x) => x.id === sessionId),
  );
  return { isActive, session, preview };
}
