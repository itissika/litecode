import { describe, expect, it } from "vitest";

import { toolTitle } from "./toolTitle";

describe("toolTitle", () => {
  it("puts content search targets ahead of execution options", () => {
    expect(
      toolTitle("grep", {
        regex: "TODO",
        path: "web/src",
        include_pattern: "**/*.tsx",
        offset: 20,
      }).summary,
    ).toBe("/TODO/ · in web/src");
    expect(
      toolTitle("glob", { pattern: "**/*.rs", path: "src" }).summary,
    ).toBe("**/*.rs · in src");
    expect(
      toolTitle("code_search", {
        query: "terminal status indicator",
        include_pattern: "**/*.tsx",
      }).summary,
    ).toBe("terminal status indicator · in **/*.tsx");
  });

  it("identifies both LSP action and target file", () => {
    expect(
      toolTitle("lsp", {
        action: "goToDefinition",
        file_path: "src/main.rs",
        line: 42,
        text: "run_server",
      }).summary,
    ).toBe("def · src/main.rs:42 run_server");
    expect(
      toolTitle("lsp", { action: "diagnostics", file_path: "web/src/App.tsx" }).summary,
    ).toBe("diag · web/src/App.tsx");
  });

  it("uses confirmed todo and plan results instead of verbose input", () => {
    expect(
      toolTitle(
        "todo",
        { todos: [{ content: "ship", status: "in_progress" }] },
        "OK. Status — pending: 2, in_progress: 1, completed: 3",
      ).summary,
    ).toBe("1 active · 2 pending · 3 done");
    expect(
      toolTitle(
        "plan",
        { action: "create", content: "# Plan" },
        "Created plan at .litecode/plan/calm-river.md\nPlan filename was auto-generated; content saved.",
      ).summary,
    ).toBe(".litecode/plan/calm-river.md");
    expect(
      toolTitle("plan", { action: "create" }, undefined, {
        activePlanPath: ".litecode/plan/active.md",
      }).summary,
    ).toBe(".litecode/plan/active.md");
  });

  it("uses the command as the bash title (description is optional)", () => {
    expect(
      toolTitle("bash", { command: "cargo test --workspace", workdir: "web" }).summary,
    ).toBe("cargo test --workspace");
    // description is not in the bash schema; the command wins even if present.
    expect(
      toolTitle("bash", { command: "git status", description: "Check repo state" }).summary,
    ).toBe("git status");
  });

  it("keeps a deterministic readable fallback", () => {
    expect(toolTitle("custom", { offset: 0, query: "later" }).summary).toBe('query="later"');
  });

  it("surfaces the MCP server id and first string arg", () => {
    expect(toolTitle("mcp_filesystem", { path: "/tmp", recursive: true }).summary).toBe(
      "filesystem · /tmp",
    );
    expect(toolTitle("mcp_filesystem", { recursive: true, depth: 2 }).summary).toBe(
      "filesystem",
    );
  });
});
