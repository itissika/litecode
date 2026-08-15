type MermaidApi = typeof import("mermaid").default;

import { buildMermaidInit } from "../theme/mermaid";
import { getTheme } from "./theme";

let mermaidPromise: Promise<MermaidApi> | null = null;
let initializedTheme: "dark" | "light" | null = null;

function buildInit(theme: "dark" | "light") {
  return buildMermaidInit(theme);
}

/**
 * Load (and once-initialize) mermaid with the current app theme's tokens.
 * Passing `theme` re-initializes mermaid if the active theme differs, so a
 * diagram rendered after a theme switch picks up the new palette. Mermaid
 * reads themeVariables from CSS, so var()/color-mix references also resolve
 * live without a rebuild.
 */
export function loadMermaid(theme?: "dark" | "light"): Promise<MermaidApi> {
  const target = theme ?? (getTheme() === "light" ? "light" : "dark");

  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((mod) => {
      mod.default.initialize(buildInit(target));
      initializedTheme = target;
      return mod.default;
    });
    return mermaidPromise;
  }

  return mermaidPromise.then((mod) => {
    if (target !== initializedTheme) {
      mod.initialize(buildInit(target));
      initializedTheme = target;
    }
    return mod;
  });
}

export const MERMAID_LANGS = new Set(["mermaid", "mmd"]);

export function isMermaidLang(lang: string): boolean {
  return MERMAID_LANGS.has(lang.toLowerCase());
}

/**
 * Mermaid's mindmap defaults to a radial (`cose-bilkent`) layout that scatters
 * nodes outward from the center. The conventional mindmap is a one-directional
 * tree. The renderer only honors a non-default layout through per-diagram
 * frontmatter (`config.layout`), and it reads the *global* `layout` (not the
 * `mindmap.layoutAlgorithm` field), so we can't set it globally without also
 * affecting flowcharts.
 *
 * The only tree layout registered in this mermaid build is `dagre` (the same
 * one flowcharts use). We inject it for mindmaps authored without frontmatter.
 * Note: mermaid's mindmap renderer hard-codes its direction to `TB`, so this
 * yields a top-down tree rather than a left-to-right one. Diagrams that already
 * declare frontmatter are returned untouched so the author's intent wins.
 */
export function withMindmapTreeLayout(code: string): string {
  const trimmed = code.trimStart();
  if (!/^mindmap\b/i.test(trimmed)) return code;
  return `---\nconfig:\n  layout: dagre\n---\n${code}`;
}
