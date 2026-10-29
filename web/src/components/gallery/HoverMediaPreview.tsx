import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { SpeakerHigh, SpeakerSlash } from "@phosphor-icons/react";

import { readBytes, type MediaEntry } from "../../api/workspace";
import i18n from "../../i18n";

/**
 * Budget for one hover card, not a server limit: the tab preview happily opens
 * a much larger file, but a stray hover must not pull hundreds of megabytes.
 * Uncompressed audio alone can reach tens of megabytes, so it is generous.
 */
const HOVER_PREVIEW_BYTES = 96 * 1024 * 1024;
const CARD_WIDTH = 264;
const CARD_HEIGHT = 190;
const AUDIO_HEIGHT = 84;
const GAP = 10;
const MARGIN = 8;

interface HoverMediaPreviewProps {
  entry: MediaEntry;
  /** Viewport box of the tile this card belongs to. */
  anchor: DOMRect;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
}

/**
 * Hover card for a playable tile: it fetches the bytes once and loops them, so
 * a glance is enough to tell two clips apart.
 *
 * Autoplay policy: audible playback needs a click, so a video starts muted when
 * the browser refuses and keeps a sound toggle. Audio keeps its native controls
 * — silently looping an audio file would show nothing useful.
 *
 * The card is portaled to the body: the grid scrolls and clips its own content.
 */
export function HoverMediaPreview({
  entry,
  anchor,
  onPointerEnter,
  onPointerLeave,
}: HoverMediaPreviewProps) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [muted, setMuted] = useState(false);
  const mediaRef = useRef<HTMLMediaElement | null>(null);

  const tooLarge = entry.size > HOVER_PREVIEW_BYTES;

  useEffect(() => {
    if (tooLarge) return;
    let cancelled = false;
    let created: string | null = null;
    void readBytes(entry.path).then(
      (blob) => {
        if (cancelled) return;
        created = URL.createObjectURL(blob);
        setUrl(created);
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
      if (created) URL.revokeObjectURL(created);
    };
  }, [entry.path, entry.mtime, tooLarge]);

  // Start audible; when the browser refuses, mute and retry so the picture
  // still moves. Audio stays as-is: its controls are the affordance.
  useEffect(() => {
    const element = mediaRef.current;
    if (!url || !element || entry.kind === "audio") return;
    void element.play().catch(() => {
      element.muted = true;
      setMuted(true);
      void element.play().catch(() => {});
    });
  }, [url, entry.kind]);

  const toggleSound = () => {
    const element = mediaRef.current;
    if (!element) return;
    const next = !element.muted;
    element.muted = next;
    setMuted(next);
    if (!next) void element.play().catch(() => {});
  };

  // Above the tile when there is room, otherwise below it; always inside the
  // viewport, because the card must not push a scrollbar into existence.
  const height = entry.kind === "audio" ? AUDIO_HEIGHT : CARD_HEIGHT;
  const maxLeft = Math.max(MARGIN, window.innerWidth - CARD_WIDTH - MARGIN);
  const left = Math.min(
    Math.max(MARGIN, anchor.left + anchor.width / 2 - CARD_WIDTH / 2),
    maxLeft,
  );
  const above = anchor.top - height - GAP >= MARGIN;
  const maxTop = Math.max(MARGIN, window.innerHeight - height - MARGIN);
  const top = Math.min(
    above ? anchor.top - height - GAP : anchor.bottom + GAP,
    maxTop,
  );

  return createPortal(
    <div
      className="gallery-hover"
      data-testid="gallery-hover"
      style={{ left, top, width: CARD_WIDTH }}
      onMouseEnter={onPointerEnter}
      onMouseLeave={onPointerLeave}
    >
      <div
        className={
          entry.kind === "audio"
            ? "gallery-hover-media is-audio"
            : "gallery-hover-media"
        }
      >
        {tooLarge || failed ? (
          <span className="gallery-hover-note">
            {i18n.t("gallery.previewUnavailable")}
          </span>
        ) : !url ? (
          <span className="gallery-hover-note">{i18n.t("gallery.loading")}</span>
        ) : entry.kind === "audio" ? (
          <audio
            ref={(node) => {
              mediaRef.current = node;
            }}
            src={url}
            controls
            autoPlay
            loop
            aria-label={entry.name}
            onError={() => setFailed(true)}
          />
        ) : (
          <video
            ref={(node) => {
              mediaRef.current = node;
            }}
            src={url}
            autoPlay
            loop
            playsInline
            muted={muted}
            aria-label={entry.name}
            onError={() => setFailed(true)}
          />
        )}
      </div>
      <div className="gallery-hover-caption">
        <span className="gallery-hover-name">{entry.name}</span>
        {entry.kind === "video" && url && !tooLarge && !failed ? (
          <button
            type="button"
            className="gallery-hover-sound"
            aria-label={i18n.t("gallery.sound")}
            title={i18n.t("gallery.sound")}
            onClick={toggleSound}
          >
            {muted ? <SpeakerSlash size={12} /> : <SpeakerHigh size={12} />}
          </button>
        ) : null}
      </div>
    </div>,
    document.body,
  );
}
