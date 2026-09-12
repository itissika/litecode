import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { SubagentSendToolView } from "./SubagentSendToolView";

afterEach(() => {
  cleanup();
});

describe("SubagentSendToolView", () => {
  it("shows which child session was resumed", () => {
    render(
      <SubagentSendToolView
        name="subagent_send"
        status="ok"
        input={{ id: "child-a", message: "continue" }}
        output={{
          type: "function_call_output",
          call_id: "send_a",
          output: "status: running",
        }}
      />,
    );
    expect(screen.getByText("sent to child-a")).toBeTruthy();
  });

  it("surfaces a failed send", () => {
    render(
      <SubagentSendToolView
        name="subagent_send"
        status="failed"
        input={{ id: "child-a", message: "continue" }}
      />,
    );
    expect(screen.getByText("send failed")).toBeTruthy();
  });
});
