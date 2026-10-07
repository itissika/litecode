import { cleanup, render, screen } from "@testing-library/react";
import type { IDockviewHeaderActionsProps } from "dockview-react";
import { afterEach, describe, expect, it } from "vitest";

import { BrowserHeaderAction } from "./BrowserHeaderAction";

afterEach(() => {
  cleanup();
  delete window.litecode;
});

function headerProps(type: "grid" | "edge"): IDockviewHeaderActionsProps {
  return {
    location: type === "grid" ? { type: "grid" } : { type: "edge", position: "left" },
    group: {
      api: {
        id: "group-1",
        location: type === "grid" ? { type: "grid" } : { type: "edge", position: "left" },
      },
    },
  } as IDockviewHeaderActionsProps;
}

describe("BrowserHeaderAction", () => {
  it("offers a new browser on a desktop grid tab bar", () => {
    window.litecode = { browserCreate: async () => ({}) as never };
    render(<BrowserHeaderAction {...headerProps("grid")} />);
    expect(screen.getByRole("button", { name: "New Browser" })).toBeTruthy();
  });

  it("stays off edge rails and off a browser workbench", () => {
    window.litecode = { browserCreate: async () => ({}) as never };
    const { rerender } = render(<BrowserHeaderAction {...headerProps("edge")} />);
    expect(screen.queryByRole("button", { name: "New Browser" })).toBeNull();
    delete window.litecode;
    rerender(<BrowserHeaderAction {...headerProps("grid")} />);
    expect(screen.queryByRole("button", { name: "New Browser" })).toBeNull();
  });
});
