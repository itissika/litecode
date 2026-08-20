/**
 * Ticket 07: FoldCard / projection live state is per-seq, not session turn.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

const FORBIDDEN = [
  "turnActive",
  "isLastBubble",
  "assistant-after:",
] as const;

function walkTsFiles(dir: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const st = statSync(path);
    if (st.isDirectory()) {
      walkTsFiles(path, out);
    } else if (/\.(ts|tsx)$/.test(name)) {
      if (name.includes("death-list") || name.includes(".test.")) continue;
      out.push(path);
    }
  }
}

describe("session-seq D death list — FoldCard live is seq status", () => {
  it("bans session-turn live dialect under web/src", () => {
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

  it("gate file itself is named for session-seq D", () => {
    expect(basename(__filename)).toBe("session-seq-d-death-list.test.ts");
  });
});
