import { describe, expect, it } from "vitest";
import { jobExitDetail, readableCompactSummary } from "./transcriptMarks";

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

describe("jobExitDetail", () => {
  it("reads the exit line of the background-exit reminder", () => {
    expect(
      jobExitDetail(
        "<system-reminder>\nBackground bash bg_a exited with code 3.\noutput_file: .litecode/bash/bg_a.output\ncommand: sleep 8\n</system-reminder>",
      ),
    ).toBe("bg_a · exit code 3");
  });

  it("reads the user-Kill variant", () => {
    expect(
      jobExitDetail(
        "<system-reminder>\nThe user stopped background bash bg_b (Kill).\nexit_code: 137\noutput_file: .litecode/bash/bg_b.output\n</system-reminder>",
      ),
    ).toBe("bg_b · stopped by user (Kill)");
  });

  it("returns undefined when the body carries no exit line", () => {
    expect(jobExitDetail("plain reminder body")).toBeUndefined();
    expect(jobExitDetail("")).toBeUndefined();
  });
});
