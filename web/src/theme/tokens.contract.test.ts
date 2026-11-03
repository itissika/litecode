import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const themeDir = dirname(fileURLToPath(import.meta.url));
const webSrc = join(themeDir, "..");
const desktopSrc = join(webSrc, "../../desktop/src");
const tokensPath = join(themeDir, "tokens.css");

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) walk(path, out);
    else if (/\.(css|ts|tsx)$/.test(entry)) out.push(path);
  }
  return out;
}

function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
}

function tokenNames(text: string, re: RegExp): string[] {
  const names: string[] = [];
  for (const match of stripComments(text).replaceAll("\\/", "/").matchAll(re)) {
    names.push(match[1]);
  }
  return names;
}

const files = [...walk(webSrc), ...walk(desktopSrc)];
const tokensCss = readFileSync(tokensPath, "utf8");
const definitions = new Set(tokenNames(tokensCss, /--_dk-([A-Za-z0-9/-]+)\s*:/g));

/** Local override hooks are intentionally supplied by a component, not theme tokens. */
const LOCAL_HOOKS = new Set([
  "foldcard-frame",
  "cache-pulse-color",
  "knowledge-card-fill",
  "knowledge-tone",
]);
/** Existing tokens that a component deliberately re-binds inline. */
const LOCAL_OVERRIDES = new Set(["composer-card-shadow"]);

function unresolved(file: string): string[] {
  const text = readFileSync(file, "utf8");
  return tokenNames(
    text,
    /(?:var\(\s*|\(\s*)--_dk-([A-Za-z0-9/-]+)/g,
  ).filter((name) => !definitions.has(name) && !LOCAL_HOOKS.has(name));
}

describe("theme token contract", () => {
  it("resolves every CSS var / Tailwind token reference", () => {
    const missing = new Map<string, Set<string>>();
    for (const file of files) {
      for (const name of unresolved(file)) {
        if (!missing.has(name)) missing.set(name, new Set());
        missing.get(name)!.add(file);
      }
    }
    expect(
      [...missing.entries()].map(([name, where]) => `${name} <- ${[...where].join(", ")}`),
    ).toEqual([]);
  });

  it("defines --_dk-* tokens only in tokens.css", () => {
    const outside: string[] = [];
    for (const file of files) {
      if (file === tokensPath) continue;
      const text = readFileSync(file, "utf8");
      const names = tokenNames(text, /--_dk-([A-Za-z0-9/-]+)\s*:/g).filter(
        (name) => !LOCAL_HOOKS.has(name) && !LOCAL_OVERRIDES.has(name),
      );
      if (names.length > 0) outside.push(`${file}: ${names.join(", ")}`);
    }
    expect(outside).toEqual([]);
  });

  it("keeps the theme-bound token set in the light override", () => {
    const light = tokensCss.slice(tokensCss.indexOf(':root[data-dv-theme="light"]'));
    const required = [
      "header",
      "root",
      "editor",
      "side",
      "sidepanel",
      "float",
      "overlay",
      "text-primary",
      "text-body",
      "text-secondary",
      "text-muted",
      "text-disabled",
      "accent",
      "accent-glow",
      "accent-ring",
      "accent-hover",
      "accent-halo",
      "accent-solid",
      "accent-solid-text",
      "line",
      "line-visible",
      "border-visible",
      "border-strong",
      "elevation",
      "surface-raised",
      "surface-header",
      "close-hover-color",
      "close-hover-bg",
      "hover",
      "pressed",
    ];
    const lightDefs = new Set(tokenNames(light, /--_dk-([A-Za-z0-9/-]+)\s*:/g));
    expect(required.filter((name) => !lightDefs.has(name))).toEqual([]);
  });

  it("exposes the primary semantic aliases through Tailwind @theme", () => {
    const index = readFileSync(join(themeDir, "index.css"), "utf8");
    for (const name of [
      "--color-dk-text-body",
      "--color-dk-border-strong",
      "--color-dk-accent-solid",
      "--color-dk-accent-solid-text",
      "--shadow-dk-elevation",
    ]) {
      expect(index).toContain(name);
    }
  });

  it("keeps the JS token bridge backed by real CSS variables", () => {
    const bridge = readFileSync(join(themeDir, "tokens.ts"), "utf8");
    const refs = tokenNames(bridge, /"--_dk-([A-Za-z0-9/-]+)"/g);
    expect(refs.length).toBeGreaterThan(0);
    expect(refs.filter((name) => !definitions.has(name))).toEqual([]);
  });
});
