import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { SubagentReadOnlyContent } from "../../components/SubagentReadOnlyContent";
import { releaseSessionTab } from "../../components/sessionTeardown";
import { openSessionPanel } from "../../lib/sessionPanelNav";
import { useConnectionStore } from "../../stores/connectionStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";

/**
 * Read-only dock tab for a child (subagent) session.
 *
 * It owns the exact same connection lifecycle as `AgentPanel` — (re)subscribe on
 * every "connected" transition, close on `session not found`, release on real
 * unmount — but its BODY is `SubagentReadOnlyContent`: full transcript, no
 * composer / permission / status line / model controls, user bubbles inert.
 */
export function SubagentReadOnlyPanel(props: IDockviewPanelProps) {
  const sessionId = (props.params as { sessionId?: string }).sessionId ?? "";
  const connState = useConnectionStore((s) => s.state);
  const [isActive, setIsActive] = useState(props.api.isActive);

  useEffect(() => {
    const d = props.api.onDidActiveChange((e) => setIsActive(e.isActive));
    return () => d.dispose();
  }, [props.api]);

  // (Re)subscribe while the socket is usable — re-armed on every reconnect.
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
          props.api.close();
        }
      });
    return () => {
      disposed = true;
    };
  }, [props.api, sessionId, connState]);

  // Tear down the subscription + local projection only on real unmount.
  useEffect(() => {
    if (!sessionId) return;
    return () => {
      releaseSessionTab(sessionId);
    };
  }, [sessionId]);

  const preview = useSessionStore(
    (s) => s.sessions.find((x) => x.id === sessionId)?.preview?.trim() ?? "",
  );
  // If the id was opened before the session list classified it and the list
  // later proves it is a ROOT, do not silently trap it in a read-only panel.
  // We stay fail-closed (read-only) and offer an explicit escape hatch instead
  // of auto-migrating, so a writable panel never co-exists with this one:
  // close THIS panel first, then (once it has fully unmounted) open the
  // writable one — otherwise the non-refcounted `ensureSubscribe` could be
  // armed by the new panel and immediately torn down by this panel's unmount.
  const session = useSessionStore((s) =>
    s.sessions.find((x) => x.id === sessionId),
  );
  const knownRoot = session !== undefined && !session.parent_session_id;
  const openWritable = () => {
    props.api.close();
    window.setTimeout(() => openSessionPanel(sessionId), 0);
  };
  useEffect(() => {
    props.api.setTitle(preview || sessionId.slice(0, 8));
  }, [props.api, sessionId, preview]);

  if (!sessionId) {
    return (
      <div className="flex h-full items-center justify-center px-4 text-sm text-(--_dk-text-muted)">
        No session
      </div>
    );
  }

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      {knownRoot && (
        <div className="flex shrink-0 items-center gap-2 border-b border-(--_dk-line) px-3 py-1.5 text-xs text-(--_dk-text-muted)">
          <span>Read-only view — this session is not a subagent.</span>
          <button
            type="button"
            data-testid="open-writable-session"
            onClick={openWritable}
            className="rounded px-1.5 py-0.5 font-mono text-dk-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover)"
          >
            Open writable session
          </button>
        </div>
      )}
      <SubagentReadOnlyContent sessionId={sessionId} isActive={isActive} />
    </div>
  );
}
