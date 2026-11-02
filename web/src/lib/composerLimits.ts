/**
 * Message-size guard for the human composer.
 *
 * The backend can still turn a very large first message into a compaction
 * request even when the session has no history. That path is a provider /
 * storage edge, not a useful conversation, so the UI refuses to start it.
 *
 * These are deliberately character counts, not tokens: the composer has no
 * exact tokenizer. They are ceilings, not estimates of the selected model's
 * context window; the backend remains the authority for token limits.
 */

/** A message above this size cannot be sent or queued. */
export const MAX_MESSAGE_CHARS = 50_000;

/** A draft above this size is discarded instead of being held by the editor. */
export const MAX_DRAFT_CHARS = 100_000;

const formatted = (value: number) => value.toLocaleString("en-US");

export const MESSAGE_TOO_LONG = `Message is too long (max ${formatted(MAX_MESSAGE_CHARS)} characters)`;

export const DRAFT_TOO_LONG = `Draft cleared: messages over ${formatted(MAX_DRAFT_CHARS)} characters are not kept`;

export function messageTooLong(text: string): boolean {
  return text.length > MAX_MESSAGE_CHARS;
}

export function draftTooLong(text: string): boolean {
  return text.length > MAX_DRAFT_CHARS;
}
