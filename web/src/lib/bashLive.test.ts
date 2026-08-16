import { describe, expect, it } from "vitest";

import { formatElapsed, isRunningStatusText, matchJob, parseBashId } from "./bashLive";

describe("formatElapsed", () => {
  it("formats seconds and minutes", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(12_000)).toBe("12s");
    expect(formatElapsed(83_000)).toBe("1m23s");
  });
});

describe("parseBashId / matchJob", () => {
  const jobs = [
    {
      id: "bg_a",
      call_id: "call_a",
      command_preview: "sleep",
      output_file: ".litecode/bash/bg_a.output",
      started_at_ms: 1,
    },
    {
      id: "bg_b",
      call_id: "",
      command_preview: "echo",
      output_file: ".litecode/bash/bg_b.output",
      started_at_ms: 2,
    },
  ];

  it("matches call_id first", () => {
    expect(matchJob(jobs, "call_a", "")?.id).toBe("bg_a");
  });

  it("falls back to bash_id in sealed output", () => {
    expect(
      matchJob(jobs, "missing", "status: running\nbash_id: bg_b\n")?.id,
    ).toBe("bg_b");
  });

  it("parses bash_id lines", () => {
    expect(parseBashId("status: running\nbash_id: bg_z\n")).toBe("bg_z");
    expect(isRunningStatusText("status: running\nbash_id: bg_z\n")).toBe(true);
    expect(isRunningStatusText("exit_code: 0\nhello\n")).toBe(false);
  });
});
