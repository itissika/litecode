/**
 * Hand-off of recalled text into a session's composer draft.
 *
 * The queued batch lives in the transcript area (an in-flight bubble) but the
 * editable text lives in the composer's local state, so recalling a queued
 * message needs a bridge between the two. Keeping it a module-level bus leaves
 * both sides simple: the sender never touches React state, and the composer
 * only ever appends — never replaces what the user was already writing.
 */
type ComposerAppendHandler = (sessionId: string, text: string) => void;

const handlers = new Set<ComposerAppendHandler>();

/** Append text to the composer of `sessionId` (no-op for an empty session). */
export function appendComposerText(sessionId: string, text: string): void {
  if (!text.trim()) return;
  for (const handler of handlers) handler(sessionId, text);
}

export function subscribeComposerAppend(
  handler: ComposerAppendHandler,
): () => void {
  handlers.add(handler);
  return () => {
    handlers.delete(handler);
  };
}
