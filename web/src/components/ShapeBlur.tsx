import type { CSSProperties } from "react";

/**
 * EXPERIMENTAL — concept test for "shape-controlled gradient blur".
 *
 * A uniform backdrop-blur whose visibility is faded by a shaped mask, so the
 * blur can be a soft-edged circle / ellipse / band instead of a hard-bordered
 * box. The blur radius stays constant; the mask gradient only modulates how
 * much of the blurred layer shows at each point.
 *
 * Deliberately independent of ProgressiveBlur so this experiment can be tuned
 * or deleted without touching the production blur.
 *
 * Usage: place inside a positioned ancestor, e.g.
 *   <div className="relative">
 *     <ShapeBlur shape="radial" size={40} inset={{ left: 8, top: 8 }} strength={6} />
 *   </div>
 */
export interface ShapeBlurProps {
  /** Uniform blur radius in px (applied via backdrop-filter). */
  strength?: number;
  /** Mask shape controlling how/where the blur fades.
   *  "radial" = soft-edged circle, "ellipse" = elliptical, "linear" = band. */
  shape?: "radial" | "ellipse" | "linear";
  /** Main-axis size (px): diameter for radial/ellipse, band length for linear. */
  size?: number;
  /** CSS insets within the nearest positioned ancestor. */
  inset?: { top?: number; left?: number; right?: number; bottom?: number };
  /** % of the core that stays fully blurred before fading to transparent. */
  maskSolid?: number;
  /** Optional tint colour painted behind the blur (masked with it). */
  tintColor?: string;
  /** Tint opacity (0–1), 0 = off. */
  tint?: number;
  /** Overall opacity (0–1). */
  opacity?: number;
  /** Draw a dashed outline of the blur region — handy while testing shapes. */
  debug?: boolean;
  /** Additional CSS classes. */
  className?: string;
}

function buildMask(
  shape: NonNullable<ShapeBlurProps["shape"]>,
  maskSolid: number,
): string {
  const solid = Math.max(0, Math.min(100, maskSolid));
  switch (shape) {
    case "ellipse":
      // closest-side ties the gradient to the box (width/height = size), so
      // the fade ends exactly at the edges — farthest-corner (the default)
      // would exceed the box and read as a rounded-square silhouette.
      return `radial-gradient(ellipse closest-side at center, black 0%, black ${solid}%, transparent 100%)`;
    case "linear":
      return `linear-gradient(to top, black 0%, black ${solid}%, transparent 100%)`;
    case "radial":
    default:
      // Same rationale as ellipse: circle radius = size/2, transparent right at
      // the box edge so the blur stays a clean circle.
      return `radial-gradient(circle closest-side at center, black 0%, black ${solid}%, transparent 100%)`;
  }
}

export function ShapeBlur({
  strength = 6,
  shape = "radial",
  size = 56,
  inset = {},
  maskSolid = 0,
  tintColor = "var(--_dk-editor)",
  tint = 0,
  opacity = 1,
  debug = false,
  className = "",
}: ShapeBlurProps) {
  const mask = buildMask(shape, maskSolid);

  const boxStyle: CSSProperties = {
    position: "absolute",
    width: shape === "linear" ? "100%" : size,
    height: size,
    ...inset,
  };

  return (
    <>
      {tint > 0 && (
        <div
          aria-hidden
          className={`pointer-events-none absolute ${className}`.trim()}
          style={{
            ...boxStyle,
            background: tintColor,
            maskImage: mask,
            WebkitMaskImage: mask,
            opacity: tint * opacity,
          }}
        />
      )}
      <div
        aria-hidden
        className={`pointer-events-none absolute ${className}`.trim()}
        style={{
          ...boxStyle,
          backdropFilter: `blur(${strength}px)`,
          WebkitBackdropFilter: `blur(${strength}px)`,
          maskImage: mask,
          WebkitMaskImage: mask,
          opacity,
          ...(debug
            ? { border: "1px dashed rgba(255,255,255,0.5)", borderRadius: shape === "linear" ? 0 : 9999 }
            : {}),
        }}
      />
    </>
  );
}
