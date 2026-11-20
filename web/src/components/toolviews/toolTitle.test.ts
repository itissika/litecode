import { describe, expect, it } from "vitest";

import { toolTitle } from "./toolTitle";

describe("toolTitle", () => {
  it("defers run_code header summary to RunCodeStatus", () => {
    expect(toolTitle("run_code", { code: "return 1;" }).summary).toBe("");
  });

  it("uses generic scalar-field fallback for legacy or unknown tool calls", () => {
    expect(toolTitle("custom", { offset: 0, query: "later" }).summary).toBe(
      "0",
    );
    expect(
      toolTitle("mcp_filesystem", { path: "/tmp", recursive: true }).summary,
    ).toBe("/tmp");
    expect(toolTitle("edit", { file_path: "src/a.ts" }).summary).toBe(
      "src/a.ts",
    );
  });
});
