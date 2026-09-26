import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { FunctionCallOutputItem } from "../../api/types";
import { WorkspaceToolView } from "./WorkspaceToolView";

afterEach(() => {
  cleanup();
});

function output(text: string): FunctionCallOutputItem {
  return { type: "function_call_output", call_id: "call_1", output: text };
}

const PANEL = [
  "# Workspace",
  "",
  "## mcp",
  "- `docs` · running · on for you · workspace",
  "",
  "---",
  "",
  "# Buttons",
  "",
  "| command | popup |",
  "|---|---|",
  "| `status` | again |",
].join("\n");

describe("WorkspaceToolView", () => {
  it("renders the tool text with the assistant markdown component", () => {
    render(
      <WorkspaceToolView
        name="litecode_workspace"
        status="ok"
        input={{}}
        output={output(PANEL)}
      />,
    );
    const view = screen.getByTestId("workspace-view");
    expect(view.className).toContain("tool-card-markdown");
    expect(view.querySelector(".agent-markdown")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Workspace" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "mcp" })).toBeTruthy();
    expect(view.querySelector("hr")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "command" })).toBeTruthy();
    expect(screen.getByText("status")).toBeTruthy();
  });

  it("renders guide prose the same way, including text that is not the panel", () => {
    render(
      <WorkspaceToolView
        name="litecode_workspace"
        status="ok"
        input={{ action: "guide excludes" }}
        output={output("### `excludes.json`\n\n打开工作区时种子。\n")}
      />,
    );
    expect(screen.getByRole("heading", { name: "excludes.json" })).toBeTruthy();
    expect(screen.getByText("打开工作区时种子。")).toBeTruthy();
  });

  it("keeps a failed call in markdown, colored as an error", () => {
    render(
      <WorkspaceToolView
        name="litecode_workspace"
        status="failed"
        input={{ action: "statuz" }}
        output={output("Error: unknown command 'statuz'")}
      />,
    );
    const view = screen.getByTestId("workspace-view");
    expect(view.className).toContain("text-(--_dk-red-500)");
    expect(view.textContent).toContain("statuz");
  });
});
