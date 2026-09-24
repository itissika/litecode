import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const grantPermission = vi.fn();

vi.mock("../../components/AgentChatInput", () => ({
  AgentChatInput: () => <div data-testid="chat-input" />,
}));

vi.mock("../../components/SessionStatusLine", () => ({
  SessionStatusLine: () => <div data-testid="session-status-line" />,
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

describe("ComposerDock flex height clamp", () => {
  it("fills the pane, bottom-aligns the column and keeps the shrink chain open", () => {
    render(<ComposerDock sessionId="session-1" />);

    // The dock's box is exactly the pane (`absolute inset-0`) with its column
    // bottom-aligned, so the browser — not a JS measurement — clamps the status
    // panel: the panel is the one shrinkable item, everything else is
    // `shrink-0`.
    const dock = screen.getByTestId("composer-dock");
    expect(dock.className).toContain("absolute");
    expect(dock.className).toContain("inset-0");
    expect(dock.className).toContain("justify-end");

    // Every level between the pane and the panel must let a shrink through
    // (`min-h-0`), or a long plan would push the input out of the pane instead
    // of scrolling inside the panel.
    const content = screen.getByTestId("composer-dock-content");
    const chain = [
      dock,
      dock.firstElementChild as HTMLElement,
      content,
      content.firstElementChild as HTMLElement,
    ];
    for (const el of chain) expect(el.className).toContain("min-h-0");
  });
});

describe("ComposerDock collapse", () => {
  it("folds the whole dock (permission card included) into the bar and flips the toggle", async () => {
    const user = userEvent.setup();
    render(<ComposerDock sessionId="session-1" />);

    // Expanded: permission card + input visible, toggle offers collapse.
    expect(screen.getByTestId("permission-card")).toBeTruthy();
    expect(screen.getByTestId("chat-input")).toBeTruthy();
    expect(screen.getByTestId("composer-dock-content").dataset.collapsed).toBe(
      "false",
    );
    expect(
      screen.getByRole("button", { name: "Collapse composer" }),
    ).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Collapse composer" }));

    // Collapsed: content slid out (permission card included), no fake bar,
    // toggle flips to expand.
    expect(screen.getByTestId("composer-dock-content").dataset.collapsed).toBe(
      "true",
    );
    expect(screen.queryByTestId("composer-collapsed-bar")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Expand composer" }),
    ).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Expand composer" }));

    // Expanded again.
    expect(screen.getByTestId("composer-dock-content").dataset.collapsed).toBe(
      "false",
    );
    expect(
      screen.getByRole("button", { name: "Collapse composer" }),
    ).toBeTruthy();
  });
});
