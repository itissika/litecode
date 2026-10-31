import { useEffect, useState } from "react";
import type { DockviewPanelApi } from "dockview-react";

import type { SessionInfo } from "../../api/types";
import { releaseSessionTab } from "../../components/sessionTeardown";
import { sessionTitle } from "../../lib/sessionDisplay";
import { useConnectionStore } from "../../stores/connectionStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";
import { useUnreadStore } from "../../stores/unreadStore";

/**
 * Connection lifecycle shared by the session dock panels (writable
 * `AgentPanel` and read-only `SubagentReadOnlyPanel`):
 * - tracks the dockview active state;
 * - (re)subscribes on every transition to "connected", including reconnects,
 *   so a dropped subscription self-heals;
 * - closes the panel with a toast when the session no longer exists (any
 *   other failure waits for the next "connected" transition);
 * - releases the subscription and local projection only on real unmount.
 * Returns the active flag plus the session's list entry and display name.
 */
export function useSessionPanelBootstrap(
  api: DockviewPanelApi,
  sessionId: string,
): { isActive: boolean; session: SessionInfo | undefined; title: string } {
  const connState = useConnectionStore((s) => s.state);
  const [isActive, setIsActive] = useState(api.isActive);

  useEffect(() => {
    const d = api.onDidActiveChange((e) => setIsActive(e.isActive));
    return () => d.dispose();
  }, [api]);

  // Seeing the transcript is what reads the session: the sidebar dot clears
  // only when this panel is focused, not merely rendered behind another tab.
  useEffect(() => {
    if (!isActive || !sessionId) return;
    useUnreadStore.getState().consume(sessionId);
  }, [isActive, sessionId]);

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

  const session = useSessionStore((s) =>
    s.sessions.find((x) => x.id === sessionId),
  );
  return { isActive, session, title: sessionTitle(session) };
}
