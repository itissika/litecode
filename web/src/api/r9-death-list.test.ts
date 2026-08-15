/**
 * R9 gate: product-pipeline invariants on the FE side.
 * Complements Rust death_list_gate R9 needles (#1 default Responses, #6 HookMessage ban,
 * plus R3–R6 continuity for tool_start / partialText).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");
const REPO_ROOT = join(__dirname, "../../..");

const FORBIDDEN_PRODUCTION = [
  "tool_start",
  "tool_end",
  "partialText",
  "partialReasoning",
  "liveTools",
  "HookMessage",
  "inject_messages",
  "custom_tool_to_legacy",
  "assemble_system_prompt",
  "compact_messages",
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

describe("R9 death list — FE product-pipeline invariants", () => {
  it("bans tool_start / partialText / HookMessage dialects in production adapter sources", () => {
    const files: string[] = [];
    walkTsFiles(WEB_SRC, files);
    const violations: string[] = [];
    for (const file of files) {
      const src = readFileSync(file, "utf8");
      const rel = relative(WEB_SRC, file);
      for (const needle of FORBIDDEN_PRODUCTION) {
        if (src.includes(needle)) {
          violations.push(`${rel}: forbidden \`${needle}\``);
        }
      }
    }
    expect(violations, violations.join("\n")).toEqual([]);
  });

  it("accepts shared golden Item JSON fixtures (Rust serde authority)", () => {
    const fixturesDir = join(REPO_ROOT, "tests/fixtures/items");
    for (const name of [
      "assistant_message.json",
      "function_call.json",
      "user_message.json",
    ]) {
      const raw = readFileSync(join(fixturesDir, name), "utf8");
      const item = JSON.parse(raw) as { type: string };
      expect(item.type, name).toBeTruthy();
      expect(["message", "function_call", "reasoning"]).toContain(item.type);
    }
  });

  it("gate file itself is named for R9", () => {
    expect(basename(__filename)).toBe("r9-death-list.test.ts");
  });
});
