import { FilmStrip, Images, Waveform } from "@phosphor-icons/react";
import type React from "react";

import type { AppView } from "../../api/apps";

type Glyph = React.FC<{
  size?: number;
  weight?: "thin" | "light" | "regular" | "bold";
}>;

/**
 * Built-ins LiteCode paints itself have no brand to show. They name a Phosphor
 * glyph instead, drawn inline so it follows the surrounding text color.
 */
const INLINE_GLYPHS: Record<string, Glyph> = {
  images: Images,
  filmstrip: FilmStrip,
  waveform: Waveform,
};

export function AppIcon({
  app,
  size = 32,
  className = "",
}: {
  app: Pick<AppView, "icon" | "name">;
  size?: number;
  className?: string;
}) {
  if (app.icon.startsWith("phosphor:")) {
    const Glyph = INLINE_GLYPHS[app.icon.slice("phosphor:".length)];
    if (Glyph) {
      return (
        <span
          className={`apps-icon apps-icon-emoji ${className}`}
          style={{ width: size, height: size }}
          aria-hidden="true"
        >
          <Glyph size={Math.round(size * 0.8)} weight="regular" />
        </span>
      );
    }
  }
  if (app.icon.startsWith("data:image/")) {
    return (
      <img
        src={app.icon}
        alt=""
        width={size}
        height={size}
        className={`apps-icon ${className}`}
        draggable={false}
      />
    );
  }
  return (
    <span
      className={`apps-icon apps-icon-emoji ${className}`}
      style={{ fontSize: Math.round(size * 0.72), width: size, height: size }}
      aria-hidden="true"
    >
      {app.icon || "📦"}
    </span>
  );
}
