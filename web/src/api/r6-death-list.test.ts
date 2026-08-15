/**
 * R6 gate: persistence/RPC must stay isomorphic with Item transcript.
 * Ban old messages-table / revert_messages dialect under web/src.
 * Complements Rust death_list_gate FORBIDDEN_R6_PERSISTENCE_RPC.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

const FORBIDDEN = [
  "revert_messages",
  "RevertMessages",
  "session/revert-messages",
  "revert-messages",
  "FROM messages",
  "INTO messages",
  "TABLE messages",
  "ensure_messages_schema",
] as const;

function walkTsFiles(dir: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const st = statSync(path);
    if (st.isDirectory()) {
      walkTsFiles(path, out);
    } else if (/\.(ts|tsx)$/.test(name)) {
      if (name.includes("death-list")) continue;
      out.push(path);
    }
  }
}

describe("R6 death list — persistence/RPC isomorphic names", () => {
  it("bans revert_messages / messages-table dialect under web/src", () => {
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

  it("gate file itself is named for R6", () => {
    expect(basename(__filename)).toBe("r6-death-list.test.ts");
  });
});
