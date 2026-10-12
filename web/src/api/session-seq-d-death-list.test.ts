/**
 * Ticket 07: FoldCard / projection live state is per-seq, not session turn.
 */
import { describe, expect, it } from "vitest";
import { collectTsFiles, scanNeedles } from "../test/deathListGate";

const FORBIDDEN = ["turnActive", "isLastBubble", "assistant-after:"] as const;

describe("session-seq D death list — FoldCard live is seq status", () => {
  it("bans session-turn live dialect under web/src", () => {
    const files = collectTsFiles();
    const violations = scanNeedles(files, FORBIDDEN);
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
