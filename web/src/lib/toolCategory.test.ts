import { describe, expect, it } from "vitest";

import { isInlineCall, isInlineTool, processToolBucket } from "./toolCategory";

describe("processToolBucket", () => {
  it("maps bash and edit to dedicated buckets", () => {
    expect(processToolBucket("bash")).toBe("bash");
    expect(processToolBucket("edit")).toBe("edit");
  });

  it("excludes wait_shell and kill_shell from header counts", () => {
    expect(processToolBucket("wait_shell")).toBeNull();
    expect(processToolBucket("kill_shell")).toBeNull();
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
    expect(processToolBucket("read")).toBe("tool");
    expect(processToolBucket("grep")).toBe("tool");
  });
});

describe("isInlineTool", () => {
  it("identifies auxiliary bash-series tools", () => {
    expect(isInlineTool("wait_shell")).toBe(true);
    expect(isInlineTool("kill_shell")).toBe(true);
    expect(isInlineTool("subagent_wait")).toBe(true);
    expect(isInlineTool("subagent_stop")).toBe(true);
    expect(isInlineTool("bash")).toBe(false);
    expect(isInlineTool("subagent_launch")).toBe(true);
    expect(isInlineTool("subagent_list")).toBe(true);
  });

  it("identifies the session-mount capsules todo / plan", () => {
    expect(isInlineTool("todo")).toBe(true);
    expect(isInlineTool("plan")).toBe(true);
    expect(isInlineTool("read")).toBe(false);
  });
});

describe("isInlineCall", () => {
  it("routes a BACKGROUND bash to the one-line row and keeps foreground bash on its card", () => {
    expect(isInlineCall("bash", true)).toBe(true);
    expect(isInlineCall("bash", false)).toBe(false);
    expect(isInlineCall("bash")).toBe(false);
  });

  it("delegates every name-only family to isInlineTool", () => {
    expect(isInlineCall("todo")).toBe(true);
    expect(isInlineCall("plan")).toBe(true);
    expect(isInlineCall("wait_shell")).toBe(true);
    expect(isInlineCall("subagent_send")).toBe(true);
    expect(isInlineCall("subagent_list")).toBe(true);
    expect(isInlineCall("read")).toBe(false);
    expect(isInlineCall("edit")).toBe(false);
  });
});
