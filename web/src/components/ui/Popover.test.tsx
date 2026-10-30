import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { Popover } from "./Popover";

function boxRect(
  top: number,
  bottom: number,
  left = 0,
  right = 100,
): DOMRect {
  return {
    top,
    bottom,
    left,
    right,
    width: right - left,
    height: bottom - top,
    x: left,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

function openInPane(pane: DOMRect, at: DOMRect) {
  render(
    <div className="dv-groupview">
      <div className="dv-content-container" data-testid="pane">
        <Popover
          width={320}
          trigger={({ toggle }) => (
            <button type="button" onClick={toggle}>
              Open
            </button>
          )}
        >
          <div>Panel body</div>
        </Popover>
      </div>
    </div>,
  );
  screen.getByTestId("pane").getBoundingClientRect = () => pane;
  const trigger = screen.getByRole("button", { name: "Open" });
  trigger.getBoundingClientRect = () => at;
  trigger.parentElement!.getBoundingClientRect = () => at;
  fireEvent.click(trigger);
  const panel = document.querySelector<HTMLElement>("[data-popover-panel]");
  if (!panel) throw new Error("popover did not open");
  return { panel, trigger };
}

afterEach(() => cleanup());

describe("Popover pane bounds", () => {
  it("fits the panel inside its dock pane", () => {
    const panel = openInPane(
      boxRect(0, 400, 0, 280),
      boxRect(100, 140, 8, 84),
    ).panel;
    expect(panel.getAttribute("data-litecode-overlay")).toBe("popover");
    // 320px requested, pane 280px wide: shrink to 280 - 2 * 8.
    expect(panel.style.width).toBe("264px");
    expect(panel.style.left).toBe("8px");
    expect(panel.style.top).toBe("148px");
    // Below the tile: 260px of room minus the 8px gap and 8px margin.
    expect(panel.style.maxHeight).toBe("244px");
  });

  it("closes once the trigger scrolls out of the pane", () => {
    const { trigger } = openInPane(
      boxRect(0, 400, 0, 280),
      boxRect(100, 140, 8, 84),
    );
    const outside = boxRect(420, 460, 8, 84);
    trigger.getBoundingClientRect = () => outside;
    trigger.parentElement!.getBoundingClientRect = () => outside;
    fireEvent.scroll(window);
    expect(document.querySelector("[data-popover-panel]")).toBeNull();
  });
});
