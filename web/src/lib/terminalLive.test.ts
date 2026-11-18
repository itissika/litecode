import { describe, expect, it } from "vitest";

import type { BashJob, HumanRow } from "../api/types";
import {
  formatElapsed,
  matchJobByCallId,
  terminalCallCommandByCallId,
  terminalRevealCallId,
} from "./terminalLive";

describe("formatElapsed", () => {
  it("formats seconds and minutes", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(12_000)).toBe("12s");
    expect(formatElapsed(83_000)).toBe("1m23s");
  });
});

describe("matchJobByCallId", () => {
  const job: BashJob = {
    id: "bg_a",
    call_id: "c1",
    command_preview: "sleep 8",
    output_file: ".litecode/run/bg_a.stdout",
    started_at_ms: 1000,
  };

  it("matches solely by call_id", () => {
    expect(matchJobByCallId([job], "c1")).toBe(job);
    expect(matchJobByCallId([job], "c2")).toBeUndefined();
    expect(matchJobByCallId([job], undefined)).toBeUndefined();
  });
});

describe("terminalCallCommandByCallId & terminalRevealCallId", () => {
  const rows: HumanRow[] = [
    {
      seq: 1,
      kind: "item/tool_call",
      state: "final",
      body: {
        type: "function_call",
        id: "fc_exec",
        call_id: "c1",
        name: "exec_command",
        arguments: JSON.stringify({ cmd: "cargo test --workspace" }),
        status: "completed",
      },
    },
    {
      seq: 2,
      kind: "item/tool_call",
      state: "final",
      body: {
        type: "function_call",
        id: "fc_rc",
        call_id: "rc_1",
        name: "run_code",
        arguments: JSON.stringify({ code: "tools.exec_command({ cmd: 'sleep 1' })" }),
        status: "in_progress",
      },
    },
  ];

  it("collects full exec_command cmd strings", () => {
    const map = terminalCallCommandByCallId(rows);
    expect(map.get("c1")).toBe("cargo test --workspace");
  });

  it("resolves reveal targets for top-level exec_command and nested run_code jobs", () => {
    expect(terminalRevealCallId("c1", rows)).toBe("c1");
    expect(terminalRevealCallId("rc_1:code:0", rows)).toBe("rc_1");
    expect(terminalRevealCallId("missing", rows)).toBeUndefined();
  });
});
