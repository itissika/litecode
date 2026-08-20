/**
 * Ticket 06: no parallel identity (live-* / bufferIndex / orderProjection).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

const FORBIDDEN = [
  "bufferIndex",
  "buffer_index",
  "liveItemRowId",
  "orderProjection",
  "vacateIndex",
  "sealProjectionRow",
  "findRowByItemId",
  "findRowForSeal",
  "committed_end",
  "bufferViewStart",
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

describe("session-seq A death list — FE identity is seq", () => {
  it("bans overlay identity dialect under web/src", () => {
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

  it("gate file itself is named for session-seq A", () => {
    expect(basename(__filename)).toBe("session-seq-a-death-list.test.ts");
  });
});
