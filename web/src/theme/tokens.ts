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
  root: string;
  editor: string;
  side: string;
  sidepanel: string;
  overlay: string;
  float: string;
  textPrimary: string;
  textMuted: string;
  textDisabled: string;
  line: string;
  lineVisible: string;
  /** Accent used by Mermaid / non-CSS consumers (mirrors --_dk-accent). */
  accent: string;
  /** Danger / critical color (mirrors --_dk-red-500). */
  danger: string;
  /** Resolved UI font stack (mirrors --_dk-font-ui, var chain flattened). */
  fontUi: string;
}

/** Safe values used only if a CSS variable is somehow unreadable. */
const FALLBACKS: DkTokens = {
  root: "#0a0a0a",
  editor: "#1c1c1c",
  side: "#121212",
  sidepanel: "#181818",
  overlay: "#161616",
  float: "#020202",
  textPrimary: "#ffffff",
  textMuted: "#888888",
  textDisabled: "#646463",
  line: "rgba(255, 255, 255, 0.05)",
  lineVisible: "rgba(255, 255, 255, 0.10)",
  accent: "#2c3033",
  danger: "#ef4444",
  fontUi: "Inter, ui-sans-serif, system-ui, sans-serif",
};

function readVars(root: HTMLElement): DkTokens {
  const cs = getComputedStyle(root);
  const get = (name: string, fb: string): string => {
    const v = cs.getPropertyValue(name).trim();
    return v || fb;
  };
  return {
    root: get("--_dk-root", FALLBACKS.root),
    editor: get("--_dk-editor", FALLBACKS.editor),
    side: get("--_dk-side", FALLBACKS.side),
    sidepanel: get("--_dk-sidepanel", FALLBACKS.sidepanel),
    overlay: get("--_dk-overlay", FALLBACKS.overlay),
    float: get("--_dk-float", FALLBACKS.float),
    textPrimary: get("--_dk-text-primary", FALLBACKS.textPrimary),
    textMuted: get("--_dk-text-muted", FALLBACKS.textMuted),
    textDisabled: get("--_dk-text-disabled", FALLBACKS.textDisabled),
    line: get("--_dk-line", FALLBACKS.line),
    lineVisible: get("--_dk-line-visible", FALLBACKS.lineVisible),
    accent: get("--_dk-accent", FALLBACKS.accent),
    danger: get("--_dk-red-500", FALLBACKS.danger),
    fontUi: resolveFont(get("--_dk-font-ui", FALLBACKS.fontUi), cs),
  };
}

/**
 * getComputedStyle does not flatten `var()` references, so `--_dk-font-ui`
 * comes back as `var(--font-sans)`. Resolve one level of indirection so the
 * value is a concrete font stack that consumers (Mermaid) can use directly.
 */
function resolveFont(raw: string, cs: CSSStyleDeclaration): string {
  const m = raw.match(/^var\(\s*(.+?)\s*\)$/);
  if (m) {
    const inner = cs.getPropertyValue(m[1].trim()).trim();
    if (inner) return inner;
  }
  return raw;
}

/** Read tokens for the currently active theme from the live document. */
export function readDkTokens(root: HTMLElement = document.documentElement): DkTokens {
  return readVars(root);
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
