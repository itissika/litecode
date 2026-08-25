import type { Monaco } from "@monaco-editor/react";
import { readDkTokensForTheme, type DkTokens } from "./tokens";

export const LITECODE_MONACO_THEME_DARK = "litecode-vs-dark";
export const LITECODE_MONACO_THEME_LIGHT = "litecode-vs-light";

/**
 * Monaco `defineTheme` colors must be `#RRGGBB` / `#RRGGBBAA` only.
 * `Color.fromHex` rejects `rgba(...)` / `rgb(...)` (silently becomes red) and
 * token rules throw on non-hex — both show up as a blank/white editor on light.
 */
export function toMonacoHex(cssColor: string, fallback = "#000000"): string {
  const s = cssColor.trim();
  if (!s) return fallback;

  const hex = s.match(/^#([0-9A-Fa-f]{3,4}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$/);
  if (hex) {
    const h = hex[1];
    if (h.length === 3 || h.length === 4) {
      const expanded = h
        .split("")
        .map((c) => c + c)
        .join("");
      return `#${expanded}`;
    }
    return `#${h}`;
  }

  const rgb = s.match(
    /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)(?:\s*,\s*([\d.]+))?\s*\)$/i,
  );
  if (rgb) {
    const chan = (v: string) => {
      const n = Number(v);
      const x = n <= 1 && String(v).includes(".") ? Math.round(n * 255) : Math.round(n);
      return Math.max(0, Math.min(255, x));
    };
    const r = chan(rgb[1]);
    const g = chan(rgb[2]);
    const b = chan(rgb[3]);
    const to = (n: number) => n.toString(16).padStart(2, "0");
    if (rgb[4] !== undefined) {
      const a = Math.round(Math.max(0, Math.min(1, Number(rgb[4]))) * 255);
      return `#${to(r)}${to(g)}${to(b)}${to(a)}`;
    }
    return `#${to(r)}${to(g)}${to(b)}`;
  }

  return fallback;
}

function withAlpha(hex: string, alphaByte: number): string {
  const base = toMonacoHex(hex).slice(0, 7);
  const a = Math.max(0, Math.min(255, alphaByte)).toString(16).padStart(2, "0");
  return `${base}${a}`;
}

function monacoColorsFromTokens(t: DkTokens, mode: "dark" | "light") {
  const editor = toMonacoHex(t.editor, mode === "light" ? "#ffffff" : "#1c1c1c");
  const widget = toMonacoHex(
    mode === "light" ? t.float : t.overlay,
    mode === "light" ? "#fafafa" : "#161616",
  );
  const fg = toMonacoHex(t.textPrimary, mode === "light" ? "#000000" : "#ffffff");
  const muted = toMonacoHex(t.textMuted, mode === "light" ? "#707070" : "#888888");
  const disabled = toMonacoHex(t.textDisabled, mode === "light" ? "#a7a5a5" : "#646463");
  const line = toMonacoHex(t.line, mode === "light" ? "#0000000d" : "#ffffff0d");
  const lineVisible = toMonacoHex(
    t.lineVisible,
    mode === "light" ? "#0000001a" : "#ffffff1a",
  );

  return {
    "editor.background": editor,
    "editor.foreground": fg,
    "editorGutter.background": editor,
    "editorLineNumber.foreground": disabled,
    "editorLineNumber.activeForeground": muted,
    "editor.lineHighlightBackground": "#00000000",
    "editor.lineHighlightBorder": "#00000000",
    "editorWidget.background": widget,
    "editorWidget.border": line,
    "editorWidget.foreground": fg,
    "editor.hoverBackground": widget,
    "editor.hoverBorder": lineVisible,
    "editor.hoverForeground": fg,
    "editor.hoverStatusBarBackground": widget,
    "editorIndentGuide.background": line,
    "editorIndentGuide.activeBackground": lineVisible,
    "minimap.background": editor,
    "scrollbarSlider.background": withAlpha(disabled, 0x80),
    "scrollbarSlider.hoverBackground": withAlpha(muted, 0x80),
    "editorOverviewRuler.border": "#00000000",
  };
}

function semanticTokenRules(mode: "dark" | "light"): { token: string; foreground: string }[] {
  if (mode === "light") {
    return [
      { token: "type", foreground: "267F99" },
      { token: "class", foreground: "267F99" },
      { token: "struct", foreground: "267F99" },
      { token: "interface", foreground: "267F99" },
      { token: "enum", foreground: "267F99" },
      { token: "typeParameter", foreground: "267F99" },
      { token: "function", foreground: "795E26" },
      { token: "method", foreground: "795E26" },
      { token: "macro", foreground: "0000FF" },
      { token: "variable", foreground: "001080" },
      { token: "parameter", foreground: "001080" },
      { token: "property", foreground: "001080" },
      { token: "enumMember", foreground: "0070C1" },
      { token: "namespace", foreground: "267F99" },
      { token: "keyword", foreground: "AF00DB" },
      { token: "comment", foreground: "008000" },
      { token: "string", foreground: "A31515" },
      { token: "number", foreground: "098658" },
      { token: "operator", foreground: "000000" },
      { token: "decorator", foreground: "795E26" },
    ];
  }
  return [
    { token: "type", foreground: "4EC9B0" },
    { token: "class", foreground: "4EC9B0" },
    { token: "struct", foreground: "4EC9B0" },
    { token: "interface", foreground: "4EC9B0" },
    { token: "enum", foreground: "4EC9B0" },
    { token: "typeParameter", foreground: "4EC9B0" },
    { token: "function", foreground: "DCDCAA" },
    { token: "method", foreground: "DCDCAA" },
    { token: "macro", foreground: "569CD6" },
    { token: "variable", foreground: "9CDCFE" },
    { token: "parameter", foreground: "9CDCFE" },
    { token: "property", foreground: "9CDCFE" },
    { token: "enumMember", foreground: "4FC1FF" },
    { token: "namespace", foreground: "4EC9B0" },
    { token: "keyword", foreground: "C586C0" },
    { token: "comment", foreground: "6A9955" },
    { token: "string", foreground: "CE9178" },
    { token: "number", foreground: "B5CEA8" },
    { token: "operator", foreground: "D4D4D4" },
    { token: "decorator", foreground: "DCDCAA" },
  ];
}

/*
 * Monaco editor colors — derived from the live dark theme tokens:
 *   background: t.editor   widget bg: t.overlay
 */
export function defineMonacoThemeDark(monaco: Monaco): void {
  const t = readDkTokensForTheme("dark");
  monaco.editor.defineTheme(LITECODE_MONACO_THEME_DARK, {
    base: "vs-dark",
    inherit: true,
    rules: semanticTokenRules("dark"),
    colors: monacoColorsFromTokens(t, "dark"),
  });
}

/*
 * Light Monaco theme — derived from the live light theme tokens.
 * Colors are normalized to hex so setTheme does not blank the editor.
 */
export function defineMonacoThemeLight(monaco: Monaco): void {
  const t = readDkTokensForTheme("light");
  monaco.editor.defineTheme(LITECODE_MONACO_THEME_LIGHT, {
    base: "vs",
    inherit: true,
    rules: semanticTokenRules("light"),
    colors: monacoColorsFromTokens(t, "light"),
  });
}

/** Define (or refresh) all Monaco themes. Safe to call on every theme toggle. */
export function defineAllMonacoThemes(monaco: Monaco): void {
  defineMonacoThemeDark(monaco);
  defineMonacoThemeLight(monaco);
}

/** Apply the Monaco theme that matches the app theme name. */
export function applyMonacoThemeForApp(
  monaco: typeof import("monaco-editor"),
  appTheme: string,
): void {
  defineAllMonacoThemes(monaco);
  monaco.editor.setTheme(
    appTheme === "light" ? LITECODE_MONACO_THEME_LIGHT : LITECODE_MONACO_THEME_DARK,
  );
}
