import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { RunCodeToolView } from "./RunCodeToolView";

afterEach(() => {
  cleanup();
});

const CODE = `const result = await tools.read({ path: "src/a.ts" });
return result;`;

describe("RunCodeToolView", () => {
  it("renders the program as a code block instead of a JSON string", () => {
    const { container } = render(
      <RunCodeToolView
        name="run_code"
        status="ok"
        input={{ code: CODE, timeout_ms: 5000 }}
      />,
    );

    const view = screen.getByTestId("run-code-view");
    expect(view.querySelector(".agent-code-block")).toBeTruthy();
    expect(view.textContent).toContain(CODE);
    expect(view.textContent).not.toContain("\\n");
    expect(view.textContent).toContain("timeout_ms: 5000");
    expect(container.textContent).not.toContain('"code"');
  });

  it("renders calls, logs, and diagnostics from the structured ledger", () => {
    const payload = {
      calls: [{ seq: 1, tool: "read", outcome: "ok" }],
      logs: [{ level: "info", message: "opened" }],
      diagnostics: [{ code: "lsp_error", severity: "warning", message: "unused" }],
    };
    const output = `1. read path=a.txt ok\n\n@@run_code_data\n${JSON.stringify(payload)}`;
    render(
      <RunCodeToolView
        name="run_code"
        status="ok"
        input={{ code: CODE }}
        output={{ type: "function_call_output", call_id: "c1", output }}
      />,
    );
    expect(screen.getByTestId("run-code-calls").textContent).toContain("1. read ok");
    expect(screen.getByTestId("run-code-logs").textContent).toContain("info: opened");
    expect(screen.getByTestId("run-code-diagnostics").textContent).toContain(
      "warning lsp_error: unused",
    );
    expect(screen.queryByTestId("run-code-fallback")).toBeNull();
    expect(screen.getByTestId("run-code-view").textContent).not.toContain("@@run_code_data");
  });

  it("keeps an older text ledger when the structured marker is absent", () => {
    render(
      <RunCodeToolView
        name="run_code"
        status="ok"
        input={{ code: CODE }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output: "1. read path=notes.txt ok",
        }}
      />,
    );
    expect(screen.getByTestId("run-code-fallback").textContent).toContain(
      "1. read path=notes.txt ok",
    );
  });

  it("keeps the raw input when the program is not available", () => {
    const { container } = render(
      <RunCodeToolView name="run_code" status="running" input='{"code":' />,
    );
    expect(container.textContent).toBe('{"code":');
  });
});
