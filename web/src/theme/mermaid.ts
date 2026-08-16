/**
 * Mermaid theming.
 *
 * We render on mermaid's `base` theme and override only the three dimensions
 * that define the visual language — fills, strokes/lines, and
 * text — sourcing every value from the project's design tokens (see
 * tokens.ts / tokens.css). Nothing here is a hand-picked hex: each mermaid
 * variable maps to one of a handful of token-derived roles, so diagrams stay
 * in lockstep with the app theme (light/dark) with no second color list to
 * drift. Mermaid derives the remaining micro-colors from these roles.
 *
 * The rendered SVG is transparent and sits on the app's `--_dk-editor` surface
 * (see content.css `.agent-mermaid-wrap`), so it blends into the conversation
 * bubble.
 */

import { readDkTokensForTheme, type DkTokens } from "./tokens";

/**
 * Mermaid's color parser only accepts concrete colors (hex / rgb[a] / hsl /
 * named) — it does NOT understand `color-mix()` or `var()`. So every tint we
 * need is computed here in JS and passed as a plain `rgba()` / `rgb()` string.
 */
function parseRgb(input: string): [number, number, number] {
  const s = input.trim();
  if (s.startsWith("#")) {
    let hex = s.slice(1);
    if (hex.length === 3) hex = hex[0] + hex[0] + hex[1] + hex[1] + hex[2] + hex[2];
    return [
      parseInt(hex.slice(0, 2), 16),
      parseInt(hex.slice(2, 4), 16),
      parseInt(hex.slice(4, 6), 16),
    ];
  }
  const m = s.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
  if (m) return [parseInt(m[1], 10), parseInt(m[2], 10), parseInt(m[3], 10)];
  return [0, 0, 0];
}

/** Apply an alpha to a color, returning an `rgba(...)` string. */
function withAlpha(input: string, alpha: number): string {
  const [r, g, b] = parseRgb(input);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Linearly interpolate two colors (t = weight of `to`), returning `rgb(...)`. */
function mix(from: string, to: string, t: number): string {
  const a = parseRgb(from);
  const b = parseRgb(to);
  const r = Math.round(a[0] + (b[0] - a[0]) * t);
  const g = Math.round(a[1] + (b[1] - a[1]) * t);
  const bl = Math.round(a[2] + (b[2] - a[2]) * t);
  return `rgb(${r}, ${g}, ${bl})`;
}

/** Relative luminance (0..1) of a color, used to pick a readable text color. */
function luminance([r, g, b]: [number, number, number]): number {
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255;
}

/**
 * Build mermaid `base`-theme variables from the app's design tokens.
 *
 * Every returned color is a token (or a JS-computed tint of one), grouped
 * under the three dimensions we care about. Diagram-specific variables all
 * point at the same small set of roles, which is what keeps the result
 * consistent instead of the previous hand-tuned per-slice palettes that
 * clashed. Colors are emitted as concrete `rgb()`/`rgba()` strings because
 * mermaid's parser rejects `color-mix()` and `var()`.
 */
export function buildMermaidThemeVariables(
  tokens: DkTokens,
  darkMode: boolean,
): Record<string, string | boolean> {
  const { accent, danger, textPrimary: text, fontUi } = tokens;

  // ── Token roles — the only colors we introduce ──────────────────────────
  const surface = tokens.overlay; // node / section fill (raised above editor)
  const surfaceAlt = tokens.sidepanel; // notes, alternate sections, clusters
  const border = tokens.lineVisible; // visible node & edge borders
  const hairline = tokens.line; // subtle dividers / grid lines
  const textMuted = tokens.textMuted;
  // Soft tints = accent / danger at low opacity (mermaid can't parse color-mix).
  const accentSoft = withAlpha(accent, 0.16);
  const critSoft = withAlpha(danger, 0.16);
  const critBorder = withAlpha(danger, 0.36);

  // Categorical scale (pie slices, mindmap branches) — tints of the accent so
  // it stays inside the project palette instead of mermaid's default rainbow.
  const tint = (pct: number) => mix(accent, text, pct / 100);
  const cScale = [
    accent,
    tint(14),
    tint(28),
    tint(42),
    tint(56),
    tint(18),
    tint(34),
    tint(50),
    tint(64),
    tint(24),
    tint(40),
    tint(54),
  ];

  // Mindmap needs a per-section label color (cScaleLabel) and edge color
  // (cScaleInv), plus a root color (git0 / gitBranchLabel0). Crucially it also
  // needs THEME_COLOR_LIMIT — without it the whole mindmap style block is
  // skipped and nodes render completely unstyled.
  const onLight = darkMode ? tokens.root : text; // dark text on light fills
  const onDark = darkMode ? text : tokens.overlay; // light text on dark fills
  const labelFor = (fill: string) =>
    luminance(parseRgb(fill)) > 0.55 ? onLight : onDark;
  const cScaleLabel = cScale.map(labelFor);
  const cScaleInv = cScale; // branch-colored connectors

  const scaleVars: Record<string, string> = { THEME_COLOR_LIMIT: "12" };
  for (let i = 0; i < 12; i++) {
    scaleVars[`cScale${i}`] = cScale[i];
    scaleVars[`cScaleLabel${i}`] = cScaleLabel[i];
    scaleVars[`cScaleInv${i}`] = cScaleInv[i];
    scaleVars[`pie${i + 1}`] = cScale[i];
  }
  scaleVars.git0 = accent;
  scaleVars.gitBranchLabel0 = labelFor(accent);

  return {
    darkMode,
    background: "transparent",

    // ── Fills ──────────────────────────────────────────────────────────────
    primaryColor: surface,
    primaryBorderColor: border,
    primaryTextColor: text,
    secondaryColor: surfaceAlt,
    secondaryBorderColor: border,
    secondaryTextColor: text,
    tertiaryColor: surfaceAlt,
    tertiaryBorderColor: border,
    tertiaryTextColor: textMuted,
    mainBkg: surface,
    secondBkg: surfaceAlt,
    textColor: text,
    labelTextColor: textMuted,
    titleColor: text,
    nodeBkg: surface,
    nodeBorder: border,
    clusterBkg: surfaceAlt,
    clusterBorder: border,
    edgeLabelBackground: surface,
    labelBackground: surface,
    actorBkg: surface,
    actorBorder: border,
    actorTextColor: text,
    labelBoxBkgColor: surface,
    labelBoxBorderColor: border,
    noteBkgColor: surfaceAlt,
    noteBorderColor: border,
    noteTextColor: textMuted,
    sectionBkgColor: surface,
    sectionBkgColor2: surfaceAlt,
    altSectionBkgColor: surfaceAlt,
    stateBkg: surface,
    stateLabelColor: text,
    labelBackgroundColor: surface,
    compositeBackground: surfaceAlt,
    compositeTitleBackground: border,
    compositeBorder: border,
    altBackground: surfaceAlt,
    taskBkgColor: surface,
    taskBorderColor: border,
    taskTextColor: text,
    taskTextLightColor: textMuted,
    taskTextOutsideColor: textMuted,
    doneTaskBkgColor: hairline,
    doneTaskBorderColor: border,
    gridColor: hairline,

    // ── Lines / borders ────────────────────────────────────────────────────
    lineColor: border,
    border1: border,
    border2: hairline,
    arrowheadColor: textMuted,
    defaultLinkColor: border,
    actorLineColor: hairline,
    signalColor: border,
    signalTextColor: text,
    loopTextColor: textMuted,
    todayLineColor: accent,

    // ── Text ───────────────────────────────────────────────────────────────
    fontFamily: fontUi,
    fontSize: "13px",
    fontWeight: "400",

    // ── Shape / rendering (minimal) ───────────────────────────────────────
    useGradient: false,
    strokeWidth: darkMode ? "1.5" : "1",
    radius: "6",

    // Accent-driven highlights (activation bars, active/critical tasks)
    activationBorderColor: accent,
    activationBkgColor: accentSoft,
    sequenceNumberColor: textMuted,
    activeTaskBkgColor: accentSoft,
    critBkgColor: critSoft,
    critBorderColor: critBorder,

    // Categorical scale (pie / mindmap branches) — in-palette tints of accent.
    // Built above as `scaleVars` (cScale / cScaleLabel / cScaleInv / pie /
    // THEME_COLOR_LIMIT / git0 / gitBranchLabel0) so mindmap nodes get styled.
    ...scaleVars,
  };
}

export const LITECODE_MERMAID_INIT = {
  startOnLoad: false,
  securityLevel: "strict" as const,
  flowchart: {
    useMaxWidth: false,
    htmlLabels: true,
    curve: "basis" as const,
    padding: 14,
    nodeSpacing: 36,
    rankSpacing: 44,
  },
  sequence: { useMaxWidth: false, mirrorActors: false, wrap: true },
  gantt: { useMaxWidth: false, gridLineStartPadding: 8, barHeight: 18, barGap: 6 },
  class: { useMaxWidth: false },
  er: { useMaxWidth: false },
};

/** Build the full mermaid init config for the given app color scheme. */
export function buildMermaidInit(theme: "dark" | "light") {
  return {
    ...LITECODE_MERMAID_INIT,
    theme: "base" as const,
    themeVariables: buildMermaidThemeVariables(
      readDkTokensForTheme(theme),
      theme === "dark",
    ),
  };
}
