import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { Dropdown } from "./Dropdown";

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

function openAt(at: DOMRect, pane?: DOMRect) {
  render(
    <div className="dv-groupview">
      <div className="dv-content-container" data-testid="pane">
        <Dropdown
          direction="up"
          maxHeight={480}
          trigger={({ toggle }) => (
            <button type="button" onClick={toggle}>
              Open
            </button>
          )}
        >
          <div>Row</div>
        </Dropdown>
      </div>
    </div>,
  );
  if (pane) screen.getByTestId("pane").getBoundingClientRect = () => pane;
  const trigger = screen.getByRole("button", { name: "Open" });
  trigger.getBoundingClientRect = () => at;
  trigger.parentElement!.getBoundingClientRect = () => at;
  fireEvent.click(trigger);
  const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
  if (!panel) throw new Error("dropdown did not open");
  return panel;
}

afterEach(() => cleanup());

describe("Dropdown sizing", () => {
  it("keeps a requested max height when the open side can hold it", () => {
    const vh = window.innerHeight;
    const panel = openAt(
      boxRect(vh - 80, vh - 60),
      boxRect(0, vh, 0, window.innerWidth),
    );
    expect(panel.style.bottom).not.toBe("");
    expect(panel.style.maxHeight).toBe("480px");
  });

  it("drops a requested max height that does not fit the open side", () => {
    const panel = openAt(
      boxRect(270, 290),
      boxRect(0, 300, 0, window.innerWidth),
    );
    expect(panel.style.bottom).not.toBe("");
    expect(panel.style.maxHeight).toBe("262px");
  });

  it("clamps to the render overlay when the trigger is not inside a group", () => {
    const vh = window.innerHeight;
    expect(vh).toBeGreaterThan(700);
    render(
      <div className="dv-render-overlay" data-testid="overlay">
        <Dropdown
          direction="up"
          maxHeight={480}
          trigger={({ toggle }) => (
            <button type="button" onClick={toggle}>
              Open
            </button>
          )}
        >
          <div>Row</div>
        </Dropdown>
      </div>,
    );
    // The overlay is the pane's content box: it starts well below the screen
    // top. A window-only clamp would keep the 480px wish.
    screen.getByTestId("overlay").getBoundingClientRect = () =>
      boxRect(400, 700, 0, window.innerWidth);
    const trigger = screen.getByRole("button", { name: "Open" });
    const at = boxRect(660, 680);
    trigger.getBoundingClientRect = () => at;
    trigger.parentElement!.getBoundingClientRect = () => at;
    fireEvent.click(trigger);
    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("dropdown did not open");
    expect(panel.style.bottom).not.toBe("");
    expect(panel.style.maxHeight).toBe("252px");
  });

  it("shifts a left-anchored panel back inside the viewport", () => {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const panel = openAt(
      boxRect(vh - 80, vh - 60, vw - 20, vw - 4),
      boxRect(0, vh, 0, vw),
    );
    const left = parseFloat(panel.style.left);
    const maxWidth = parseFloat(panel.style.maxWidth);
    expect(left).toBeGreaterThanOrEqual(8);
    expect(left + maxWidth).toBeLessThanOrEqual(vw - 8);
  });
});
