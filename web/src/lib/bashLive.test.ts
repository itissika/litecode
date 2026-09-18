import { describe, expect, it } from "vitest";

import type { HumanRow } from "../api/types";
import {
  bashCallMetaByCallId,
  formatElapsed,
  headExitCode,
  isBackgroundBashResult,
  isBashJobLive,
  isRunningStatusText,
  matchJob,
  parseBashId,
} from "./bashLive";

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

describe("isBackgroundBashResult / headExitCode", () => {
  const RUNNING = `status: running
bash_id: bg_a
output_file: .litecode/bash/bg_a.output
`;

  it("recognizes a sealed background-bash result from its text alone", () => {
    expect(isBackgroundBashResult(RUNNING)).toBe(true);
    // bash_id without the status word (e.g. a status document) is still a job.
    expect(isBackgroundBashResult(`bash_id: bg_a
running: 1
`)).toBe(true);
    expect(isBackgroundBashResult(`exit_code: 0
hello
`)).toBe(false);
  });

  it("reads a leading exit_code line only", () => {
    expect(headExitCode(`exit_code: 3
boom
`)).toBe(3);
    expect(headExitCode(`exit_code: 0
`)).toBe(0);
    expect(headExitCode(`note: see exit_code: 1
`)).toBeNull();
    expect(headExitCode(RUNNING)).toBeNull();
  });
});

describe("isBashJobLive", () => {
  const RUNNING = `status: running
bash_id: bg_a
`;
  const job = {
    id: "bg_a",
    call_id: "call_a",
    command_preview: "sleep",
    output_file: ".litecode/bash/bg_a.output",
    started_at_ms: 1,
  };

  it("is live only while a matching job is still in the snapshot", () => {
    expect(isBashJobLive(RUNNING, job)).toBe(true);
    // The seal is one-way: a vanished job means the process ended.
    expect(isBashJobLive(RUNNING, undefined)).toBe(false);
  });

  it("is never live for a completed document", () => {
    expect(isBashJobLive(`exit_code: 0
ok
`, job)).toBe(false);
  });
});

function callRow(
  callId: string,
  name: string,
  args: Record<string, unknown>,
  seq = 1,
): HumanRow {
  return {
    seq,
    kind: "item/tool_call",
    streaming: false,
    body: {
      type: "function_call",
      id: `fc_${callId}`,
      call_id: callId,
      name,
      arguments: JSON.stringify(args),
      status: "completed",
    },
  };
}

function outputRow(callId: string, output: string, seq = 2): HumanRow {
  return {
    seq,
    kind: "item/tool_result",
    streaming: false,
    body: { type: "function_call_output", call_id: callId, output },
  };
}

describe("bashCallMetaByCallId", () => {
  it("collects the full command and marks an explicit background call", () => {
    const meta = bashCallMetaByCallId([
      callRow("c1", "bash", { command: "npm run dev", run_in_background: true }),
    ]);
    expect(meta.get("c1")).toMatchObject({
      command: "npm run dev",
      background: true,
    });
  });

  it("leaves a foreground call un-owned until its result converts it", () => {
    const foreground = bashCallMetaByCallId([
      callRow("c1", "bash", { command: "ls" }),
    ]);
    expect(foreground.get("c1")?.background).toBe(false);

    // A foreground call that outlived its wait seals as a running document and
    // becomes a job the transcript itself renders as the single-line row.
    const converted = bashCallMetaByCallId([
      callRow("c1", "bash", { command: "cargo build" }),
      outputRow("c1", `status: running\nbash_id: bg_a\n`),
    ]);
    expect(converted.get("c1")?.background).toBe(true);
    expect(converted.get("c1")?.output?.call_id).toBe("c1");
  });

  it("ignores non-bash calls and unloaded calls", () => {
    const meta = bashCallMetaByCallId([
      callRow("c1", "read", { file_path: "a.ts", run_in_background: true }),
      outputRow("c1", "file body"),
    ]);
    expect(meta.size).toBe(0);
  });
});
