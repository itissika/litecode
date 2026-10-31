/**
 * One naming rule for a session, wherever the UI shows it.
 *
 * Session identity and session activity are deliberately different:
 * - `sessionTitle` names the row / dock tab / panel: what the user asked.
 * - `sessionActivity` describes what the agent last said, for the Workers
 *   roster, where the assistant's own words are the useful summary.
 *
 * Call sites must not inline `a || b` again — that is how the sidebar, tab,
 * roster and turn notification drifted apart.
 */

interface TitleFields {
  title?: string | null;
  preview?: string | null;
}

interface ActivityFields {
  assistant_preview?: string | null;
  preview?: string | null;
}

/**
 * Identity: a user title wins, then the last user-message preview.
 *
 * Never falls back to `assistant_preview`: the name of a session is what the
 * user typed, not what the agent answered.
 */
export function sessionTitle(session: TitleFields | undefined): string {
  return session?.title?.trim() || session?.preview?.trim() || "";
}

/**
 * What the session last said: assistant text first, then the user preview.
 * A roster label, not an identity.
 */
export function sessionActivity(session: ActivityFields | undefined): string {
  return session?.assistant_preview?.trim() || session?.preview?.trim() || "";
}
