import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { ToolIcon } from "./ToolIcon";

afterEach(() => {
  cleanup();
});

describe("ToolIcon settle animation", () => {
  it("pops when streaming transitions true→false on ok", () => {
    const { container, rerender } = render(
      <ToolIcon name="write" status="ok" streaming />,
    );
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
    rerender(<ToolIcon name="write" status="ok" streaming={false} />);
    expect(container.querySelector(".tool-icon--pop")).toBeTruthy();
  });

  it("pops with the warn colour when streaming transitions true→false on warning", () => {
    const { container, rerender } = render(
      <ToolIcon name="write" status="warning" streaming />,
    );
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
    rerender(<ToolIcon name="write" status="warning" streaming={false} />);
    expect(container.querySelector(".tool-icon--pop")).toBeTruthy();
    expect(container.querySelector(".tool-icon--warn")).toBeTruthy();
  });

  it("stays static when mounted with streaming=false (no transition)", () => {
    const { container } = render(
      <ToolIcon name="write" status="ok" streaming={false} />,
    );
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
  });

  it("plays fail animation when streaming transitions true→false on failed", () => {
    const { container, rerender } = render(
      <ToolIcon name="write" status="failed" streaming />,
    );
    expect(container.querySelector(".tool-icon--fail-anim")).toBeNull();
    rerender(<ToolIcon name="write" status="failed" streaming={false} />);
    expect(container.querySelector(".tool-icon--fail-anim")).toBeTruthy();
  });
});
