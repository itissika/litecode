import { useEffect, useState } from "react";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { SessionItem } from "./SessionItem";
import { openSessionPanel } from "../lib/sessionPanelNav";

export function SessionList() {
  const sessions = useSessionStore((s) => s.sessions);
  const loading = useSessionStore((s) => s.sessionsLoading);
  const error = useSessionStore((s) => s.sessionListError);
  const deleteSession = useSessionStore((s) => s.deleteSession);
  const newSession = useSessionStore((s) => s.newSession);
  const listSessions = useSessionStore((s) => s.listSessions);
  const connState = useConnectionStore((s) => s.state);

  // Shared wall-clock tick (1min) so each row's relative "time since update"
  // stays live at minute granularity without spawning a timer per SessionItem.
  // Minute granularity keeps re-render frequency low (once per row per minute).
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60000);
    return () => clearInterval(t);
  }, []);

  // Wait for the WS to actually be usable before pulling the list. On startup
  // the socket is still handshaking when this panel mounts, so an immediate
  // `listSessions()` would race the connect and fail with "No connected WS".
  // Gating on `connected` (and re-firing on every reconnect) makes the
  // connecting → connected transition itself the trigger — no manual Retry
  // needed for the normal path. A genuine failure *after* connect still sets
  // `error` and surfaces the Retry affordance.
  useEffect(() => {
    if (connState === "connected") {
      listSessions();
    }
  }, [connState, listSessions]);

  const connecting = connState === "connecting" || connState === "reconnecting";

  const handleSessionClick = (sessionId: string) => {
    openSessionPanel(sessionId);
  };

  const renderBody = () => {
    if (sessions.length === 0) {
      // While the socket is establishing or recovering, show a connecting
      // state rather than a misleading error/retry.
      if (connecting && !error) {
        return (
          <p className="p-4 text-center text-xs text-(--_dk-text-disabled)">
            Connecting…
          </p>
        );
      }
      if (error) {
        return (
          <div className="flex flex-col items-center gap-2 p-4 text-center">
            <p className="text-xs text-(--_dk-red-500)">{error}</p>
            <button type="button" onClick={() => listSessions()} className="btn-ghost">
              Retry
            </button>
          </div>
        );
      }
      return (
        <p className="p-4 text-center text-xs text-(--_dk-text-disabled)">
          {loading ? "Loading sessions…" : "No saved sessions yet."}
        </p>
      );
    }
    return (
      <div>
        {sessions.map((s) => (
          <SessionItem
            key={s.id}
            session={s}
            now={now}
            onOpen={handleSessionClick}
            onDelete={deleteSession}
          />
        ))}
      </div>
    );
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-3 py-2">
        <button type="button" onClick={newSession} className="btn-ghost btn-sm">
          New
        </button>
        <div className="flex min-w-0 items-center gap-2">
          {error && connState === "connected" && (
            <button
              type="button"
              onClick={() => listSessions()}
              className={`btn-ghost text-(--_dk-ix-danger-fg)`}
              title={error}
            >
              Retry
            </button>
          )}
          {(loading || connecting) && (
            <span
              className="h-3 w-3 shrink-0 animate-spin rounded-full border border-(--_dk-text-muted) border-t-transparent"
              title={connecting ? "Connecting…" : "Loading sessions"}
            />
          )}
          <span className="min-w-0 truncate text-[10px] text-(--_dk-text-disabled)">
            {sessions.length} session{sessions.length !== 1 ? "s" : ""}
          </span>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {renderBody()}
      </div>
    </div>
  );
}
