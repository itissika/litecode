/**
 * Model Selection Contract gate — FE invariants.
 * @see docs/model-selection-contract.md
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { WEB_SRC, collectTsFiles, scanNeedles } from "../test/deathListGate";

const FORBIDDEN = [
  "modelOverride",
  "effectiveApiModelId",
  "resolveActiveModel",
] as const;

describe("model-selection death list", () => {
  it("bans local override / api-id fallback identity mixing", () => {
    const files = collectTsFiles(undefined, { includeTests: true });
    const violations = scanNeedles(files, FORBIDDEN);
    expect(violations, violations.join("\n")).toEqual([]);
  });

  it("ModelSwitcher has no unknown label, no connectionStore fallback, and surfaces a missing ref", () => {
    const src = readFileSync(
      join(WEB_SRC, "components/ModelSwitcher.tsx"),
      "utf8",
    );
    expect(src.includes('"unknown"')).toBe(false);
    expect(src.includes("useConnectionStore")).toBe(false);
    // A ref the active catalog doesn't know is shown, never silently replaced.
    expect(src.includes("Missing: ")).toBe(true);
  });

  it("connectionStore has no session model field", () => {
    const src = readFileSync(
      join(WEB_SRC, "stores/connectionStore.ts"),
      "utf8",
    );
    expect(/\bmodel\s*:/.test(src)).toBe(false);
    expect(src.includes("hello.model")).toBe(false);
  });
});
