import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { SubagentSendToolView } from "./SubagentSendToolView";

afterEach(() => {
  cleanup();
});

describe("SubagentSendToolView", () => {
  it("shows the child id and started status without dumping the report", () => {
    render(
      <SubagentSendToolView
        name="subagent_send"
        status="ok"
        input={{ id: "child-a", message: "continue" }}
        output={{
          type: "function_call_output",
          call_id: "send_a",
          output: "status: running\nThe child runs in the background\n",
        }}
      />,
    );
    const line = screen.getByTestId("subagent-send-line");
    expect(line.textContent).toContain("child-a");
    expect(line.textContent).toContain("continue");
    expect(line.textContent).toContain("running");
    expect(line.textContent).not.toContain("The child runs in the background");
  });

  it("surfaces a failed send", () => {
    render(
      <SubagentSendToolView
        name="subagent_send"
        status="failed"
        input={{ id: "child-a", message: "continue" }}
      />,
    );
    expect(screen.getByTestId("subagent-send-line").textContent).toContain("failed");
  });
});
