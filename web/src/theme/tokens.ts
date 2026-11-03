/**
 * Runtime access to design tokens for code that needs token values in JS
 * (Monaco theme definitions, Mermaid theme variables) rather than CSS.
 *
 * The single source of truth is tokens.css (the --_dk-* custom properties).
 * These helpers read the *live* computed values via getComputedStyle, so there
 * is never a second hand-maintained copy to keep in sync — edit a value in
 * tokens.css and every consumer updates automatically, including on theme
 * switch.
 */

export interface DkTokens {
  header: string;
  root: string;
  editor: string;
  side: string;
  sidepanel: string;
  overlay: string;
  float: string;
  surfaceRaised: string;
  surfaceHeader: string;
  textPrimary: string;
  textBody: string;
  textSecondary: string;
  textMuted: string;
  textDisabled: string;
  line: string;
  lineVisible: string;
  borderStrong: string;
  /** Accent used by Mermaid / non-CSS consumers (mirrors --_dk-accent). */
  accent: string;
  accentHover: string;
  accentRing: string;
  /** Danger / critical color (mirrors --_dk-red-500). */
  danger: string;
  /** Git diff added color (mirrors --_dk-cat-green). */
  diffAdded: string;
  /** Git diff modified color (mirrors --_dk-cat-blue). */
  diffModified: string;
  catBlue: string;
  catCyan: string;
  catGreen: string;
  catOrange: string;
  catPink: string;
  catPurple: string;
  catRed: string;
  catYellow: string;
  /** Resolved UI font stack (mirrors --_dk-font-ui). */
  fontUi: string;
}

/**
 * Last-resort values for non-DOM execution. They mirror the dark theme; the
 * live path always reads tokens.css. Keep this list limited to the bridge
 * surface so it cannot become a second theme.
 */
const FALLBACKS: DkTokens = {
  header: "#030303",
  root: "#080808",
  editor: "#1a1a1a",
  side: "#0d0d0d",
  sidepanel: "#141414",
  overlay: "#171717",
  float: "#000000",
  surfaceRaised: "#1e1e20",
  surfaceHeader: "#111114",
  textPrimary: "#f5f5f5",
  textBody: "#e0e0e0",
  textSecondary: "#a3a3a3",
  textMuted: "#787878",
  textDisabled: "#525252",
  line: "rgba(255, 255, 255, 0.05)",
  lineVisible: "rgba(255, 255, 255, 0.10)",
  borderStrong: "rgba(255, 255, 255, 0.22)",
  accent: "#2c3033",
  accentHover: "#e8ecf0",
  accentRing: "rgb(186 188 190)",
  danger: "#ef4444",
  diffAdded: "#22c55e",
  diffModified: "#1a85ff",
  catBlue: "#1a85ff",
  catCyan: "#06b6d4",
  catGreen: "#22c55e",
  catOrange: "#f97316",
  catPink: "#ec4899",
  catPurple: "#a855f7",
  catRed: "#ef4444",
  catYellow: "#eab308",
  fontUi: '"Inter", ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
};

function readVars(root: HTMLElement): DkTokens {
  const cs = getComputedStyle(root);
  const get = (name: string, fb: string): string => {
    const v = cs.getPropertyValue(name).trim();
    return v || fb;
  };
  return {
    header: get("--_dk-header", FALLBACKS.header),
    root: get("--_dk-root", FALLBACKS.root),
    editor: get("--_dk-editor", FALLBACKS.editor),
    side: get("--_dk-side", FALLBACKS.side),
    sidepanel: get("--_dk-sidepanel", FALLBACKS.sidepanel),
    overlay: get("--_dk-overlay", FALLBACKS.overlay),
    float: get("--_dk-float", FALLBACKS.float),
    surfaceRaised: get("--_dk-surface-raised", FALLBACKS.surfaceRaised),
    surfaceHeader: get("--_dk-surface-header", FALLBACKS.surfaceHeader),
    textPrimary: get("--_dk-text-primary", FALLBACKS.textPrimary),
    textBody: get("--_dk-text-body", FALLBACKS.textBody),
    textSecondary: get("--_dk-text-secondary", FALLBACKS.textSecondary),
    textMuted: get("--_dk-text-muted", FALLBACKS.textMuted),
    textDisabled: get("--_dk-text-disabled", FALLBACKS.textDisabled),
    line: get("--_dk-line", FALLBACKS.line),
    lineVisible: get("--_dk-line-visible", FALLBACKS.lineVisible),
    borderStrong: get("--_dk-border-strong", FALLBACKS.borderStrong),
    accent: get("--_dk-accent", FALLBACKS.accent),
    accentHover: get("--_dk-accent-hover", FALLBACKS.accentHover),
    accentRing: get("--_dk-accent-ring", FALLBACKS.accentRing),
    danger: get("--_dk-red-500", FALLBACKS.danger),
    diffAdded: get("--_dk-cat-green", FALLBACKS.diffAdded),
    diffModified: get("--_dk-cat-blue", FALLBACKS.diffModified),
    catBlue: get("--_dk-cat-blue", FALLBACKS.catBlue),
    catCyan: get("--_dk-cat-cyan", FALLBACKS.catCyan),
    catGreen: get("--_dk-cat-green", FALLBACKS.catGreen),
    catOrange: get("--_dk-cat-orange", FALLBACKS.catOrange),
    catPink: get("--_dk-cat-pink", FALLBACKS.catPink),
    catPurple: get("--_dk-cat-purple", FALLBACKS.catPurple),
    catRed: get("--_dk-cat-red", FALLBACKS.catRed),
    catYellow: get("--_dk-cat-yellow", FALLBACKS.catYellow),
    fontUi: get("--_dk-font-ui", FALLBACKS.fontUi),
  };
}

/**
 * Read tokens for a specific theme even if it is not currently active.
 * Temporarily flips the document theme attribute and restores it within the
 * same synchronous turn, so there is no visible repaint. Used to define both
 * Monaco themes up front regardless of the active theme.
 */
export function readDkTokensForTheme(theme: "dark" | "light"): DkTokens {
  if (typeof document === "undefined") return { ...FALLBACKS };
  const de = document.documentElement;
  const prev = de.getAttribute("data-dv-theme");
  de.setAttribute("data-dv-theme", theme);
  const tokens = readVars(de);
  if (prev === null) de.removeAttribute("data-dv-theme");
  else de.setAttribute("data-dv-theme", prev);
  return tokens;
}
