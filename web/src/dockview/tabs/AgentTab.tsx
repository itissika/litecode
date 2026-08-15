import type { IDockviewPanelProps } from "dockview-react";

import { useSessionStore } from "../../stores/sessionStore";
import { SessionStatusDot, deriveSessionStatus } from "../../components/SessionStatusDot";

/**
 * Tab for an open agent (session) panel. The panel header was removed because
 * the tab now carries the session identity + live status, so this tab shows a
 * status dot (vertical bar + pulse, same as SessionItem) followed by the title
 * and a hover close button.
 */
export function AgentTab(props: IDockviewPanelProps<{ sessionId?: string }>) {
  const sessionId = props.params.sessionId ?? "";
  const session = useSessionStore((s) => s.sessions.find((x) => x.id === sessionId));
  const status = deriveSessionStatus(session);
  // Subscribe to the preview directly so the tab shows the live summary instead
  // of relying on setTitle() propagating to api.title.
  const preview = (session?.preview ?? "").trim();

  return (
    <div className="flex items-center gap-1.5 px-1.5 h-full w-full group transition-colors duration-120 hover:brightness-125 active:brightness-75">
      <SessionStatusDot status={status}>
        <span className="text-xs truncate flex-1 min-w-0 select-none">
          {preview || sessionId.slice(0, 8)}
        </span>
      </SessionStatusDot>
      <button
        className="rounded p-0.5 opacity-50 hover:opacity-100 hover:text-(--_dk-red-500) transition-opacity flex-shrink-0"
        title="Close"
        onClick={(e) => {
          e.stopPropagation();
          props.api.close();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 15 15" fill="currentColor">
          <path d="M11.78 3.22a.75.75 0 0 1 0 1.06L8.06 8l3.72 3.72a.75.75 0 1 1-1.06 1.06L7 9.06l-3.72 3.72a.75.75 0 0 1-1.06-1.06L5.94 8 2.22 4.28a.75.75 0 0 1 1.06-1.06L7 6.94l3.72-3.72a.75.75 0 0 1 1.06 0Z" />
        </svg>
      </button>
    </div>
  );
}
