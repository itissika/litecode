/**
 * Litecode brand mark: the "L" of the wordmark, traced from the logo font
 * (`Logo.tsx` renders "LiteCode" in Lexend Deca 600) into an inline path, so a
 * 12px tool row needs no webfont download.
 *
 * Numbers come straight from `LexendDeca-600.ttf`: unitsPerEm 1000, cap height
 * 700, stem 150, arm 135 — a plain right-angled outline. Scaled by 0.03 into a
 * 24-box (cap height 21 ≈ the 87% artwork height of the Phosphor glyphs it sits
 * beside) and centred horizontally. Re-trace if the wordmark font ever changes.
 */
const L_PATH = "M19.01 22.5 L5 22.5 L5 1.5 L9.5 1.5 L9.5 18.45 L19.01 18.45 Z";

export interface LitecodeMarkProps {
  size?: number;
  /** Accepted for parity with the Phosphor glyphs; the mark is always solid. */
  weight?: string;
  className?: string;
}

export function LitecodeMark({ size = 24, className }: LitecodeMarkProps) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d={L_PATH} />
    </svg>
  );
}
