/**
 * Hand-off of recalled text into a session's composer draft.
 *
 * The queued batch lives in the transcript area (an in-flight bubble) but the
 * editable text lives in the composer's local state, so recalling a queued
 * message needs a bridge between the two. Keeping it a module-level bus leaves
 * both sides simple: the sender never touches React state, and the composer
 * only ever appends — never replaces what the user was already writing.
 */
type ComposerAppendHandler = (
  sessionId: string,
  text: string,
  images: string[],
) => void;

const handlers = new Set<ComposerAppendHandler>();
const mountedComposers = new Set<string>();
let lastFocusedComposer: string | null = null;

/** The chat box that should receive an editor selection. */
export function composerTarget(): string | null {
  if (lastFocusedComposer && mountedComposers.has(lastFocusedComposer)) {
    return lastFocusedComposer;
  }
  const first = mountedComposers.values().next();
  return first.done ? null : first.value;
}

export function noteComposerFocus(sessionId: string): void {
  lastFocusedComposer = sessionId;
}

export function registerComposer(sessionId: string): () => void {
  mountedComposers.add(sessionId);
  return () => {
    mountedComposers.delete(sessionId);
    if (lastFocusedComposer === sessionId) lastFocusedComposer = null;
  };
}

/** Append text (and any recalled images) to the composer of `sessionId`. */
export function appendComposerText(
  sessionId: string,
  text: string,
  images: string[] = [],
): void {
  if (!text.trim() && images.length === 0) return;
  for (const handler of handlers) handler(sessionId, text, images);
}

export function subscribeComposerAppend(
  handler: ComposerAppendHandler,
): () => void {
  handlers.add(handler);
  return () => {
    handlers.delete(handler);
  };
}
