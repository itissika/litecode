import type { MessageSource } from "../../api/types";
import { isBotSourced } from "../../lib/botMessage";
import { MentionText } from "../mention/MentionText";
import { ImageThumb } from "../ImageThumb";
import { BotSourceMark } from "./BotSourceMark";

/**
 * The queued batch, rendered where its durable message will land.
 *
 * It is not a log row: it wears the same markup as a durable user bubble (same
 * padding, bullet and text classes) so nothing moves when the real row
 * arrives — only a veil on the text says "not written yet". Clicking pulls the
 * whole batch back into the composer, but only while the server still holds it;
 * once it has been claimed the message is durable and recall would be a lie.
 *
 * A batch with a bot sender wears the bot's mark instead, and is never
 * recallable: a person cannot take a bot's message back.
 */
export function PendingQueueBubble({
  text,
  images = [],
  sources = [],
  canRecall,
  onRecall,
}: {
  text: string;
  images?: string[];
  /** Bot senders across the batch; any one makes the bubble bot-attributed. */
  sources?: MessageSource[];
  canRecall: boolean;
  onRecall: () => void;
}) {
  const body = (
    <>
      {images.length > 0 ? (
        <div className="mb-1 flex flex-wrap gap-1.5">
          {images.map((ref, index) => (
            <ImageThumb key={`${ref}:${index}`} mediaRef={ref} />
          ))}
        </div>
      ) : null}
      {text ? <MentionText text={text} /> : null}
    </>
  );
  if (isBotSourced(sources)) {
    // Same markup as the durable bot row: accent bullet and sender mark, veiled
    // until it lands. No recall affordance: a bot's message is never withdrawn.
    return (
      <div className="py-4">
        <div
          data-pending-queue-bubble
          data-pending-bot
          className="queued-bubble-enter flex items-start gap-2"
        >
          <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-accent-hover)" />
          <div className="min-w-0 flex-1 opacity-60">
            <BotSourceMark sources={sources}>{body}</BotSourceMark>
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="py-4">
      <div
        data-pending-queue-bubble
        role={canRecall ? "button" : undefined}
        tabIndex={canRecall ? 0 : undefined}
        aria-label={canRecall ? "Recall queued message" : undefined}
        onClick={canRecall ? onRecall : undefined}
        onKeyDown={
          canRecall
            ? (event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                onRecall();
              }
            : undefined
        }
        className={`queued-bubble-enter group flex items-start gap-2 ${
          canRecall ? "cursor-pointer" : ""
        }`}
      >
        {/* Same bullet as a durable user bubble: the bubble differs only by the
            veil, so nothing shifts when the real row takes over. */}
        <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-accent-hover)" />
        {/* Veil on the text only: dimming the glyph too made the bullet read
            as a rendering artifact rather than a "not written yet" cue. */}
        <div
          className={`text-dk-base min-w-0 flex-1 text-(--_dk-text-primary) pl-(--_dk-indent-card-head) opacity-60 transition-opacity duration-200 ${
            canRecall ? "group-hover:opacity-100" : ""
          }`}
        >
          {body}
        </div>
      </div>
    </div>
  );
}
