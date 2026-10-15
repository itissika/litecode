import { useEffect } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { SubagentReadOnlyContent } from "../../components/SubagentReadOnlyContent";
import { openSessionPanel } from "../../lib/sessionPanelNav";
import { useSessionPanelBootstrap } from "./useSessionPanelBootstrap";

/**
 * Read-only dock tab for a child (subagent) session.
 *
 * It shares `AgentPanel`'s connection lifecycle (`useSessionPanelBootstrap`)
 * but its BODY is `SubagentReadOnlyContent`: full transcript, no
 * composer / permission / status line / model controls, user bubbles inert.
 */
export function SubagentReadOnlyPanel(props: IDockviewPanelProps) {
  const sessionId = (props.params as { sessionId?: string }).sessionId ?? "";
  const { isActive, session, preview } = useSessionPanelBootstrap(
    props.api,
    sessionId,
  );
  // If the id was opened before the session list classified it and the list
  // later proves it is a ROOT, do not silently trap it in a read-only panel.
  // We stay fail-closed (read-only) and offer an explicit escape hatch instead
  // of auto-migrating, so a writable panel never co-exists with this one:
  // close THIS panel first, then (once it has fully unmounted) open the
  // writable one — otherwise the non-refcounted `ensureSubscribe` could be
  // armed by the new panel and immediately torn down by this panel's unmount.
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
