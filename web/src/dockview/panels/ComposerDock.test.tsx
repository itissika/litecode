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
              kind: "permission",
              free_text: false,
              options: [],
              multi_select: false,
              questions: [],
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

  it("wears the dock's own glass — no stripe, no colour of its own", () => {
    render(<ComposerDock sessionId="session-1" />);
    const cls = screen.getByTestId("permission-card").className;
    // composerCardClass glass: rounded-md + translucent fill + 12px backdrop blur.
    expect(cls).toContain("rounded-md");
    expect(cls).toContain("backdrop-blur-[12px]");
    // The ask is carried by copy + buttons: no accent stripe, no tint.
    expect(cls).not.toContain("border-l-2");
    expect(cls).not.toContain("amber");
  });

  it("wires Allow once to grantPermission", async () => {
    const user = userEvent.setup();
    render(<ComposerDock sessionId="session-1" />);
    await user.click(screen.getByRole("button", { name: "Allow once" }));
    // No free-text field on this ask, so the receipt carries an undefined opinion.
    expect(grantPermission).toHaveBeenCalledWith(
      "session-1",
      true,
      false,
      undefined,
    );
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
    const content = screen.getByTestId("composer-dock-content");
    const toggle = screen.getByRole("button", { name: "Expand composer" });
    expect(content.dataset.collapsed).toBe("true");
    expect(screen.queryByTestId("composer-collapsed-bar")).toBeNull();
    // The wrapper keeps its layout box (the slide is a transform), so it must
    // drop pointer events too — otherwise the band the composer occupied keeps
    // swallowing the wheel and the transcript cannot scroll there.
    const wrapper = content.parentElement as HTMLElement;
    expect(wrapper.className).toContain("pointer-events-none");
    expect(toggle.className).toContain("pointer-events-auto");
    // And it must stay absolutely positioned while collapsed. A `relative` here
    // wins over `absolute` (Tailwind emits .relative after .absolute) and drops
    // the toggle back into the wrapper's flex flow, growing it by the button's
    // 20px — the bottom-aligned composer then jumps up for a frame before the
    // slide starts (the "bounce up on collapse").
    expect(toggle.className).toContain("absolute");
    expect(toggle.className).not.toContain("relative");

    await user.click(screen.getByRole("button", { name: "Expand composer" }));

    // Expanded again — and the wrapper is hit-testable again.
    expect(screen.getByTestId("composer-dock-content").dataset.collapsed).toBe(
      "false",
    );
    expect((content.parentElement as HTMLElement).className).toContain(
      "pointer-events-auto",
    );
    expect(
      screen.getByRole("button", { name: "Collapse composer" }),
    ).toBeTruthy();
  });

  it("publishes the collapse state so the transcript can drop its bottom pad", async () => {
    const onCollapsedChange = vi.fn();
    const user = userEvent.setup();
    render(
      <ComposerDock
        sessionId="session-1"
        onCollapsedChange={onCollapsedChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Collapse composer" }));
    expect(onCollapsedChange).toHaveBeenLastCalledWith(true);

    await user.click(screen.getByRole("button", { name: "Expand composer" }));
    expect(onCollapsedChange).toHaveBeenLastCalledWith(false);
  });
});
