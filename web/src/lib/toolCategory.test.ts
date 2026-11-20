import { describe, expect, it } from "vitest";

import { processToolBucket } from "./toolCategory";

describe("processToolBucket", () => {
  it("maps exec_command, write_stdin, and edit to dedicated buckets", () => {
    expect(processToolBucket("exec_command")).toBe("bash");
    expect(processToolBucket("write_stdin")).toBe("bash");
    expect(processToolBucket("edit")).toBe("edit");
  });

  it("excludes subagent rows from header counts", () => {
    expect(processToolBucket("subagent_wait")).toBeNull();
    expect(processToolBucket("subagent_stop")).toBeNull();
    expect(processToolBucket("subagent_launch")).toBeNull();
    expect(processToolBucket("subagent_list")).toBeNull();
  });

  it("excludes the session-mount capsules (todo / plan) from header counts", () => {
    expect(processToolBucket("todo")).toBeNull();
    expect(processToolBucket("plan")).toBeNull();
  });

  it("groups remaining tools under tool", () => {
    expect(processToolBucket("run_code")).toBe("tool");
    expect(processToolBucket("read")).toBe("tool");
    expect(processToolBucket("grep")).toBe("tool");
  });
});
