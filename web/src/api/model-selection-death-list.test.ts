/**
 * Model Selection Contract gate — FE invariants.
 * @see docs/model-selection-contract.md
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

const FORBIDDEN = [
  "modelOverride",
  "effectiveApiModelId",
  "resolveActiveModel",
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

describe("model-selection death list", () => {
  it("bans local override / api-id fallback identity mixing", () => {
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

  it("ModelSwitcher has no unknown label and no connectionStore model fallback", () => {
    const src = readFileSync(
      join(WEB_SRC, "components/ModelSwitcher.tsx"),
      "utf8",
    );
    expect(src.includes('"unknown"')).toBe(false);
    expect(src.includes("useConnectionStore")).toBe(false);
    expect(src.includes("modelId")).toBe(true);
  });

  it("connectionStore has no session model field", () => {
    const src = readFileSync(join(WEB_SRC, "stores/connectionStore.ts"), "utf8");
    expect(/\bmodel\s*:/.test(src)).toBe(false);
    expect(src.includes("hello.model")).toBe(false);
  });
});
