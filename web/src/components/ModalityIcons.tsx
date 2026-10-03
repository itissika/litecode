import {
  FilePdfIcon,
  ImageIcon,
  SpeakerHighIcon,
  VideoIcon,
} from "@phosphor-icons/react";

/**
 * Input modalities as 12px glyphs, in the catalog's own order
 * (`Modality::ALL` in Rust, minus `text`) so every row scans the same way.
 *
 * `text` is deliberately absent: every model must declare it
 * (`resolve_model` rejects a catalog without it), so a text glyph would sit on
 * every row identically and say nothing. The glyphs mark what a model takes
 * *beyond* text, and a row with none stays name plus switch.
 *
 * The wire sends a closed set of tokens, so one the table does not name is
 * dropped rather than given a guess of an icon.
 */
const MODALITY_GLYPHS: { token: string; Glyph: typeof ImageIcon }[] = [
  { token: "image", Glyph: ImageIcon },
  { token: "video", Glyph: VideoIcon },
  { token: "audio", Glyph: SpeakerHighIcon },
  { token: "pdf", Glyph: FilePdfIcon },
];

/**
 * What a model takes *beyond* text, as glyphs. Shared by the settings model
 * list and the composer's model switcher so both read the same way.
 *
 * Renders nothing for a text-only model, and nothing when the handshake
 * carried no `modalities` at all (older server) — an unknown catalog must not
 * be dressed up as "text only".
 */
export function ModalityIcons({ modalities }: { modalities?: string[] }) {
  const present = MODALITY_GLYPHS.filter(({ token }) =>
    modalities?.includes(token),
  );
  if (present.length === 0) return null;
  return (
    <span
      className="flex shrink-0 items-center gap-1 text-(--_dk-text-muted)"
      aria-label="Input modalities"
    >
      {present.map(({ token, Glyph }) => (
        <span
          key={token}
          role="img"
          aria-label={token}
          title={`Accepts ${token} input`}
          className="inline-flex"
        >
          <Glyph size={12} aria-hidden />
        </span>
      ))}
    </span>
  );
}
