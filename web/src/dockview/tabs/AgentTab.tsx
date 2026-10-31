import type { IDockviewPanelProps } from "dockview-react";

import { useSessionStore } from "../../stores/sessionStore";
import { useTurnStore } from "../../stores/turnStore";
import { fallbackSessionTitle } from "../../lib/sessionPanelNav";
import { sessionTitle } from "../../lib/sessionDisplay";
import {
  SessionStatusDot,
  deriveSessionStatus,
} from "../../components/SessionStatusDot";
import { BaseTab, TabTitle } from "./BaseTab";

/**
 * Tab for an open agent (session) panel. The panel header was removed because
 * the tab now carries the session identity + live status, so this tab shows a
 * status dot (vertical bar + pulse, same as SessionItem) followed by the title
 * and a hover close button.
 */
export function AgentTab(
  props: IDockviewPanelProps<{ sessionId?: string; sessionKind?: string }>,
) {
  const sessionId = props.params.sessionId ?? "";
  const session = useSessionStore((s) =>
    s.sessions.find((x) => x.id === sessionId),
  );
  const pendingCount = useTurnStore(
    (s) => s.byId.get(sessionId)?.permissionQueue.length ?? 0,
  );
  const status = deriveSessionStatus(session, {
    pendingPermissionCount: pendingCount,
  });
  // Subscribe to the session row directly so the tab shows the live name
  // instead of relying on setTitle() propagating to api.title.
  const title = sessionTitle(session);
  // Same rule as AgentPanel's writability: a known non-child session, or a
  // trusted root entry (a freshly created session the list has not seen yet).
  // A blank root reads "NEW"; a child or unclassified id keeps its short id.
  const rootPanel =
    !session?.parent_session_id &&
    (session !== undefined || props.params.sessionKind === "root");

  return (
    <BaseTab onClose={() => props.api.close()}>
      <SessionStatusDot status={status}>
        <TabTitle>
          {title || fallbackSessionTitle(sessionId, rootPanel)}
        </TabTitle>
      </SessionStatusDot>
    </BaseTab>
  );
}
