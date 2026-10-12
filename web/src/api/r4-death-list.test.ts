/**
 * R4 gate: ToolStart/ToolEnd / liveTools must not return under web/src.
 * Complements Rust `death_list_gate` (ToolStarted / WireEvent::ToolStart / …).
 */
import { describe, expect, it } from "vitest";
import { collectTsFiles, scanNeedles } from "../test/deathListGate";

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

describe("R4 death list — no ToolStart/liveTools bypass", () => {
  it("bans tool_start/tool_end/liveTools needles under web/src (non-test)", () => {
    const files = collectTsFiles();
    const violations = scanNeedles(files, FORBIDDEN);
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
