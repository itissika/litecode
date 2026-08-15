/**
 * R5 gate: partialText / partialReasoning dual-path dialect must not return under web/src.
 * Complements R4 liveTools ban. Exclude this file / other death-list tests from self-match.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

const FORBIDDEN = ["partialText", "partialReasoning"] as const;

function walkTsFiles(dir: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const st = statSync(path);
    if (st.isDirectory()) {
      walkTsFiles(path, out);
    } else if (/\.(ts|tsx)$/.test(name)) {
      // Exclude death-list gate files themselves (they mention banned needles).
      if (name.includes("death-list")) continue;
      out.push(path);
    }
  }
}

describe("R5 death list — no partialText/partialReasoning dual path", () => {
  it("bans partialText/partialReasoning under web/src", () => {
    const files: string[] = [];
    walkTsFiles(WEB_SRC, files);
    const violations: string[] = [];
    for (const file of files) {
      const src = readFileSync(file, "utf8");
      const rel = relative(WEB_SRC, file);
      for (const needle of FORBIDDEN) {
        if (src.includes(needle)) {
          violations.push(`${rel}: forbidden \`${needle}\``);
        }
      }
    }
    expect(violations, violations.join("\n")).toEqual([]);
  });

  it("gate file itself is named for R5", () => {
    expect(basename(__filename)).toBe("r5-death-list.test.ts");
  });
});
