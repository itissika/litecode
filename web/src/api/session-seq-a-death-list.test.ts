/**
 * Ticket 06: no parallel identity (live-* / bufferIndex / orderProjection).
 */
import { describe, expect, it } from "vitest";
import { collectTsFiles, scanNeedles } from "../test/deathListGate";

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

describe("session-seq A death list — FE identity is seq", () => {
  it("bans overlay identity dialect under web/src", () => {
    const files = collectTsFiles();
    const violations = scanNeedles(files, FORBIDDEN);
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
