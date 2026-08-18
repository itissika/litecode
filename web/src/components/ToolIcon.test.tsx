import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { ToolIcon } from "./ToolIcon";

afterEach(() => {
  cleanup();
});

describe("ToolIcon settle animation", () => {
  it("pops when a live group lands on ok", () => {
    const { container } = render(<ToolIcon name="write" status="ok" live />);
    expect(container.querySelector(".tool-icon--pop")).toBeTruthy();
  });

  it("stays static when remounted outside the live window", () => {
    const { container } = render(<ToolIcon name="write" status="ok" live={false} />);
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
  });
});
