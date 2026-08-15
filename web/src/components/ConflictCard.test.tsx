import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConflictCard } from "./ConflictCard";

afterEach(cleanup);

describe("ConflictCard (FE-01)", () => {
  it("shows the conflicted file and its source", () => {
    render(
      <ConflictCard path="src/a.ts" source="agent" onDismiss={() => {}} />,
    );
    expect(screen.getByTestId("conflict-card")).toBeTruthy();
    expect(screen.getByText(/unsaved changes conflicted with agent/i)).toBeTruthy();
    expect(screen.getByText("src/a.ts")).toBeTruthy();
  });

  it("fires onDismiss when the user dismisses the card", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    render(
      <ConflictCard path="src/a.ts" source="agent" onDismiss={onDismiss} />,
    );
    await user.click(screen.getByRole("button", { name: /dismiss conflict/i }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
