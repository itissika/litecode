import { describe, expect, it } from "vitest";

import { isInlineTool, processToolBucket } from "./toolCategory";

describe("processToolBucket", () => {
  it("maps bash and edit to dedicated buckets", () => {
    expect(processToolBucket("bash")).toBe("bash");
    expect(processToolBucket("edit")).toBe("edit");
  });

  it("excludes wait_shell and kill_shell from header counts", () => {
    expect(processToolBucket("wait_shell")).toBeNull();
    expect(processToolBucket("kill_shell")).toBeNull();
  });

  it("groups remaining tools under tool", () => {
    expect(processToolBucket("read")).toBe("tool");
    expect(processToolBucket("grep")).toBe("tool");
  });
});

describe("isInlineTool", () => {
  it("identifies auxiliary bash-series tools", () => {
    expect(isInlineTool("wait_shell")).toBe(true);
    expect(isInlineTool("kill_shell")).toBe(true);
    expect(isInlineTool("bash")).toBe(false);
  });
});
