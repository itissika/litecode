/**
 * R6 gate: persistence/RPC must stay isomorphic with Item transcript.
 * Ban old messages-table / revert_messages dialect under web/src.
 * Complements Rust death_list_gate FORBIDDEN_R6_PERSISTENCE_RPC.
 */
import { describe, expect, it } from "vitest";
import { collectTsFiles, scanNeedles } from "../test/deathListGate";

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

describe("R6 death list — persistence/RPC isomorphic names", () => {
  it("bans revert_messages / messages-table dialect under web/src", () => {
    const files = collectTsFiles(undefined, { includeTests: true });
    const violations = scanNeedles(files, FORBIDDEN);
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
