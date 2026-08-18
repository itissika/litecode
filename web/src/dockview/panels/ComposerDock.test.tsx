import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const grantPermission = vi.fn();

vi.mock("../../components/AgentChatInput", () => ({
  AgentChatInput: () => <div data-testid="chat-input" />,
}));

vi.mock("../../components/TodoPanel", () => ({
  TodoPanel: () => <div data-testid="todo-panel" />,
}));

vi.mock("../../components/TerminalStatusBar", () => ({
  TerminalStatusBar: () => <div data-testid="terminal-status" />,
}));

vi.mock("../../stores/turnStore", () => ({
  useTurnStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({
      byId: new Map([
        [
          "session-1",
          {
            pendingPermission: {
              turn_id: "turn-1",
              request_id: "req-abcdef12",
              tool: "bash",
              rule_id: "default",
              summary: "Run bash command",
            },
          },
        ],
      ]),
      grantPermission,
    }),
}));

import { ComposerDock } from "./AgentPanel";

afterEach(() => {
  cleanup();
  grantPermission.mockClear();
});

describe("ComposerDock permission overlay", () => {
  it("renders the permission card in the composer overlay, not as a fullscreen layer", () => {
    const { container } = render(<ComposerDock sessionId="session-1" />);
    const card = screen.getByTestId("permission-card");
    expect(container.querySelector(".fixed.inset-0")).toBeNull();
    expect(card.className).not.toMatch(/\bfixed\b/);
    expect(card.textContent).toMatch(/bash/);
    expect(card.textContent).toMatch(/Run bash command/);
  });

  it("wires Allow once to grantPermission", async () => {
    const user = userEvent.setup();
    render(<ComposerDock sessionId="session-1" />);
    await user.click(screen.getByRole("button", { name: "Allow once" }));
    expect(grantPermission).toHaveBeenCalledWith("session-1", true, false);
  });

  it("shows Latest in the overlay when the list is unstuck", async () => {
    const onJumpToEnd = vi.fn();
    const user = userEvent.setup();
    render(
      <ComposerDock
        sessionId="session-1"
        stickToEnd={false}
        onJumpToEnd={onJumpToEnd}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Latest" }));
    expect(onJumpToEnd).toHaveBeenCalled();
  });

  it("hides Latest while stuck to the end", () => {
    render(<ComposerDock sessionId="session-1" stickToEnd />);
    expect(screen.queryByRole("button", { name: "Latest" })).toBeNull();
  });
});
