import { copyFileSync, mkdirSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Copy web locale JSON (single source of truth) into desktop/dist/i18n/.
 * Mirrors copy-theme-tokens.mjs — never maintain a second dictionary under desktop/src.
 */
export function copyI18nLocales(srcDir, destDir) {
  mkdirSync(destDir, { recursive: true });
  const names = readdirSync(srcDir).filter((name) => name.endsWith(".json"));
  if (names.length === 0) {
    throw new Error(`no locale JSON files in ${srcDir}`);
  }
  const copied = [];
  for (const name of names) {
    const from = path.join(srcDir, name);
    const to = path.join(destDir, name);
    copyFileSync(from, to);
    copied.push({ from, to });
  }
  return copied;
}

const isMain =
  Boolean(process.argv[1]) &&
  path.resolve(fileURLToPath(import.meta.url)) === path.resolve(process.argv[1]);

if (isMain) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const src = path.resolve(root, "../web/src/i18n/locales");
  const dest = path.join(root, "dist", "i18n");
  const copied = copyI18nLocales(src, dest);
  for (const { from, to } of copied) {
    console.log(`copied ${from} -> ${to}`);
  }
}
