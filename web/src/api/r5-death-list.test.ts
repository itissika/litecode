/**
 * R5 gate: partialText / partialReasoning dual-path dialect must not return under web/src.
 * Complements R4 liveTools ban. Exclude this file / other death-list tests from self-match.
 */
import { describe, expect, it } from "vitest";
import { collectTsFiles, scanNeedles } from "../test/deathListGate";

const FORBIDDEN = ["partialText", "partialReasoning"] as const;

describe("R5 death list — no partialText/partialReasoning dual path", () => {
  it("bans partialText/partialReasoning under web/src", () => {
    const files = collectTsFiles(undefined, { includeTests: true });
    const violations = scanNeedles(files, FORBIDDEN);
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
