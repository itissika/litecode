import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { FloatingDialog } from "./FloatingDialog";

afterEach(() => cleanup());

describe("FloatingDialog overlay marker", () => {
  it("marks the visible dialog as an occluding surface", () => {
    const { container } = render(
      <FloatingDialog visible title="Test" onClose={() => undefined}>
        <div>body</div>
      </FloatingDialog>,
    );
    expect(container.querySelector('[data-litecode-overlay="dialog"]')).toBeTruthy();
  });

  it("renders nothing when hidden", () => {
    const { container } = render(
      <FloatingDialog visible={false} title="Test" onClose={() => undefined}>
        <div>body</div>
      </FloatingDialog>,
    );
    expect(container.firstChild).toBeNull();
  });
});
