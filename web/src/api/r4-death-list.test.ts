/**
 * R4 gate: ToolStart/ToolEnd / liveTools must not return under web/src.
 * Complements Rust `death_list_gate` (ToolStarted / WireEvent::ToolStart / …).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

const FORBIDDEN = [
  "liveTools",
  "LiveToolCall",
  "applyToolStartToRow",
  "applyToolEndToRow",
  "pruneLiveToolsForItem",
  "tool_start",
  "tool_end",
  "liveMetadata",
  "liveOk",
  "liveResult",
] as const;

function walkTsFiles(dir: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const st = statSync(path);
    if (st.isDirectory()) {
      walkTsFiles(path, out);
    } else if (/\.(ts|tsx)$/.test(name) && !name.endsWith(".test.ts") && !name.endsWith(".test.tsx")) {
      out.push(path);
    }
  }
}

describe("R4 death list — no ToolStart/liveTools bypass", () => {
  it("bans tool_start/tool_end/liveTools needles under web/src (non-test)", () => {
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
});
