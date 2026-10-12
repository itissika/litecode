/**
 * Shared walk/scan for web death-list gates (WP-045 P3/P13).
 * Exemption is an exact gate-file name, not a substring match.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

/** `web/src` — helper lives in `web/src/test/`. */
export const WEB_SRC = join(__dirname, "..");

/** Exact gate filenames only (e.g. `r5-death-list.test.ts`). */
export const DEATH_LIST_GATE_RE = /^[\w-]+-death-list\.test\.ts$/;

export type CollectOptions = {
  /**
   * When false (default), skip `*.test.ts(x)` — product-only gates.
   * When true, still skip exact death-list gate files via DEATH_LIST_GATE_RE.
   */
  includeTests?: boolean;
};

export function collectTsFiles(
  root: string = WEB_SRC,
  opts: CollectOptions = {},
): string[] {
  const includeTests = opts.includeTests ?? false;
  const out: string[] = [];
  const walk = (dir: string): void => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      const st = statSync(path);
      if (st.isDirectory()) {
        walk(path);
        continue;
      }
      if (!/\.(ts|tsx)$/.test(name)) continue;
      if (DEATH_LIST_GATE_RE.test(name)) continue;
      if (!includeTests && /\.test\.(ts|tsx)$/.test(name)) continue;
      out.push(path);
    }
  };
  walk(root);
  return out;
}

/** Scan files for forbidden substrings; returns violation lines `rel: forbidden \`needle\``. */
export function scanNeedles(
  files: string[],
  needles: readonly string[],
  root: string = WEB_SRC,
  skipRel?: (rel: string) => boolean,
): string[] {
  const violations: string[] = [];
  for (const file of files) {
    const rel = relative(root, file);
    if (skipRel?.(rel)) continue;
    const src = readFileSync(file, "utf8");
    for (const needle of needles) {
      if (src.includes(needle)) {
        violations.push(`${rel}: forbidden \`${needle}\``);
      }
    }
  }
  return violations;
}
