import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useConnectionStore } from "../stores/connectionStore";
import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";
import { LlmReconnectBubble } from "./LlmReconnectBubble";

afterEach(() => {
  cleanup();
  useTurnStore.setState({ byId: new Map() });
});

describe("LlmReconnectBubble", () => {
  it("shows a spinner while reconnecting and no retry button", () => {
    useTurnStore.setState({
      byId: new Map([
        [
          "session-1",
          {
            ...EMPTY_SLICE,
            llmReconnect: {
              phase: "connecting",
              attempt: 2,
              max_attempts: 6,
            },
          },
        ],
      ]),
    });
    render(<LlmReconnectBubble sessionId="session-1" />);
    expect(screen.getByRole("status").textContent).toContain(
      "Reconnecting (2/6)",
    );
    expect(screen.queryByTestId("llm-reconnect-retry")).toBeNull();
    expect(document.querySelector(".animate-spin")).not.toBeNull();
  });

  it("sends agent/retry from the failed bubble", async () => {
    const sendRpc = vi.fn(async () => ({ started: true }));
    useConnectionStore.setState({ sendRpc } as never);
    useTurnStore.setState({
      byId: new Map([
        [
          "session-1",
          {
            ...EMPTY_SLICE,
            llmReconnect: {
              phase: "failed",
              attempt: 6,
              max_attempts: 6,
            },
          },
        ],
      ]),
    });
    render(<LlmReconnectBubble sessionId="session-1" />);
    expect(screen.getByRole("status").textContent).toContain(
      "Reconnection failed (6/6)",
    );
    await userEvent.click(screen.getByTestId("llm-reconnect-retry"));
    expect(sendRpc).toHaveBeenCalledWith("agent/retry", {
      session_id: "session-1",
    });
  });

  it("hides Retry on a child session and still shows the failure", () => {
    useTurnStore.setState({
      byId: new Map([
        [
          "child-1",
          {
            ...EMPTY_SLICE,
            llmReconnect: {
              phase: "failed",
              attempt: 6,
              max_attempts: 6,
            },
          },
        ],
      ]),
    });
    render(<LlmReconnectBubble sessionId="child-1" allowRetry={false} />);
    expect(screen.getByRole("status").textContent).toContain(
      "Reconnection failed (6/6)",
    );
    expect(screen.queryByTestId("llm-reconnect-retry")).toBeNull();
  });
});