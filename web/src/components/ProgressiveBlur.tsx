interface ProgressiveBlurProps {
  /** Total height of the blur area in pixels. Default: 44. */
  height?: number;
  /**
   * Blur radius in pixels applied across the whole band.
   * Default: round(√height × 1.2).
   */
  strength?: number;
  /**
   * Mask solid zone percentage (0–100). Bottom X% of the band is fully
   * blurred before linearly fading to transparent at the opposite edge.
   * Lower = text disappears faster. Higher = gradual fade. Default: 50.
   */
  maskSolid?: number;
  /**
   * Color tint opacity at the strongest edge (0–1).
   * Overlays tintColor to hide text behind blur.
   * 0 = off, 1 = fully opaque. Default: 0.
   */
  tint?: number;
  /**
   * CSS color for the tint overlay, e.g. "var(--_dk-editor)" or "#1c1c1c".
   * Should match the element's background so the overlay is invisible when
   * no text is beneath it. Default: "var(--_dk-editor)".
   */
  tintColor?: string;
  /**
   * Tint gradient curve (0–2). 0 = linear, 2 = quadratic.
   * Higher = color stays opaque longer at the anchor edge.
   * Default: 2.
   */
  tintCurve?: number;
  /** Which edge the blur originates from. Default "bottom". */
  side?: "bottom" | "top";
  /** Overall opacity of the blur effect (0–1). Default 1. */
  opacity?: number;
  /**
   * Pixel gap between the band and the anchor edge (top for side="top",
   * bottom for side="bottom"). Lets the band float in from the edge instead
   * of being glued to it — e.g. to leave the same breathing room the content
   * list has at its top. Default: 0.
   */
  offset?: number;
  /** Additional CSS classes. */
  className?: string;
}

export function ProgressiveBlur({
  height: rawHeight,
  strength: rawStrength,
  maskSolid: rawMaskSolid,
  tint: rawTint,
  tintColor: rawTintColor,
  tintCurve: rawTintCurve,
  side = "bottom",
  opacity = 1,
  offset = 0,
  className = "",
}: ProgressiveBlurProps) {
  const totalH = rawHeight ?? 44;
  const strength = rawStrength ?? Math.round(Math.sqrt(totalH) * 1.2);
  const maskSolid = rawMaskSolid ?? 50;
  const tint = rawTint ?? 0;
  const tintColor = rawTintColor ?? "var(--_dk-editor)";
  const tintCurve = rawTintCurve ?? 2;

  const isBottom = side === "bottom";
  const maskDir = isBottom ? "to top" : "to bottom";
  // Anchor offset from the edge (inline style wins over the old top-0/bottom-0
  // utility), so the band can float in by `offset` px instead of hugging it.
  const anchorStyle = isBottom ? { bottom: `${offset}px` } : { top: `${offset}px` };

  // Non-linear tint gradient: sample pow(t, tintCurve) at 6 points.
  const tintStops = Array.from({ length: 6 }, (_, i) => {
    const t = i / 5;
    const pct = Math.round((1 - Math.pow(t, tintCurve)) * 100);
    return `color-mix(in srgb, ${tintColor} ${pct}%, transparent)`;
  }).join(", ");
  const tintGradient = `linear-gradient(${maskDir}, ${tintStops})`;

  const mask = `linear-gradient(${maskDir}, black 0%, black ${maskSolid}%, transparent 100%)`;

  // Solid base tint at the anchor edge (bottom 33% of the band). Painted
  // FIRST so it sits behind the blur: at the very edge the backdrop-filter
  // samples this uniform (text-free) colour instead of high-contrast text,
  // which kills the 1px seam the weak blur kernel would otherwise show.
  const baseH = Math.round(totalH * 0.12);
  return (
    <>
      {tint > 0 && (
        <div
          className={`pointer-events-none absolute left-px right-px ${className}`}
          style={{
            ...anchorStyle,
            height: `${baseH}px`,
            background: tintColor,
            opacity: tint * opacity,
            transition: "opacity 200ms ease",
          }}
        />
      )}
      <div
        className={`pointer-events-none absolute left-px right-px ${className}`}
        style={{
          ...anchorStyle,
          height: `${totalH}px`,
          backdropFilter: `blur(${strength}px)`,
          WebkitBackdropFilter: `blur(${strength}px)`,
          maskImage: mask,
          WebkitMaskImage: mask,
          opacity,
          transition: "opacity 200ms ease",
        }}
      />
      {tint > 0 && (
        <div
          className={`pointer-events-none absolute left-px right-px ${className}`}
          style={{
            ...anchorStyle,
            height: `${totalH}px`,
            background: tintGradient,
            opacity: tint * opacity,
            transition: "opacity 200ms ease",
          }}
        />
      )}
    </>
  );
}
