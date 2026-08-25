import { describe, expect, it } from "vitest";
import { readableCompactSummary } from "./transcriptMarks";

describe("readableCompactSummary", () => {
  it("strips the conversation summary label prefix", () => {
    expect(readableCompactSummary("[Conversation summary]\nDone X and Y")).toBe("Done X and Y");
  });

  it("strips the aggressive summary label prefix", () => {
    expect(readableCompactSummary("[Aggressive summary]\nOnly key facts")).toBe("Only key facts");
  });

  it("removes internal system-reminder blocks", () => {
    const raw = "[Conversation summary]\n<system-reminder>\nkeep recent tool results\n</system-reminder>\nProse body";
    expect(readableCompactSummary(raw)).toBe("Prose body");
  });

  it("returns clean text untouched", () => {
    expect(readableCompactSummary("Plain summary")).toBe("Plain summary");
  });
});
