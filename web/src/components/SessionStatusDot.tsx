import type { ReactNode } from "react";

import type { SessionInfo } from "../api/types";

export type SessionStatus = "idle" | "running" | "pending";

/**
 * Single leading indicator that encodes the session status purely through
 * colour + animation — a thin vertical bar plus a rightward glow beam:
 *   - idle:    static muted bar
 *   - running: colour shift to emerald, then a green pulse that shoots out to
 *              the right (repeating lateral pulse)
 *   - pending: amber bar with a rightward glow that breathes in a loop
 *              (breathing cycle, awaiting a permission grant)
 *
 * Self-contained: when `children` are supplied the glow is rendered and anchored
 * to that content (the bar sits to its left, the glow spans from the bar across
 * the content's left third), so callers get the bar AND the animation in one
 * place. Without `children` only the bare bar is rendered.
 *
 * Pure presentational: callers derive `status` from a session payload.
 */
export function SessionStatusDot({
  status,
  children,
}: {
  status: SessionStatus;
  children?: ReactNode;
}) {
  const glow = (status === "running" || status === "pending") && (
    <span
      className={`status-bar-glow ${status === "running" ? "status-bar-glow--running" : "status-bar-glow--pending"}`}
      aria-hidden
    >
      {status === "running" && <span className="status-bar-glow__beam" />}
    </span>
  );

  // Bare bar, no content to anchor the glow to.
  if (!children) {
    return <span className={`status-bar status-bar--${status}`} />;
  }

  return (
    <span className="relative flex min-w-0 flex-1 items-center gap-2">
      <span className={`status-bar status-bar--${status}`} />
      {/* Content the glow is anchored to — relative so the absolutely-positioned
          glow (left:-8px bridges the gap-2, right:0 pins to this edge) is
          bounded by the content width and can never overrun past it. */}
      <span className="relative flex min-w-0 flex-1 items-center gap-2">
        {children}
        {glow}
      </span>
    </span>
  );
}

/** Derive the display status from a session payload. */
export function deriveSessionStatus(
  session: Pick<SessionInfo, "running" | "turn"> | undefined,
): SessionStatus {
  if (session?.turn?.awaiting_permission) return "pending";
  if (session?.running) return "running";
  return "idle";
}
