import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";
import { emptySlice, useTurnStore } from "../stores/turnStore";
import { ActivePlanChip } from "./ActivePlanChip";

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  useTurnStore.setState({ byId: new Map() });
  useSessionStore.setState({ project: null } as never);
});

describe("ActivePlanChip", () => {
  it("is absent without an active plan", () => {
    render(<ActivePlanChip sessionId="session-1" />);
    expect(screen.queryByRole("button", { name: "Open active plan" })).toBeNull();
  });

  it("opens the workspace-relative active plan", () => {
    vi.useFakeTimers();
    const openFile = vi.fn(async () => {});
    useSessionStore.setState({ project: "E:\\project" } as never);
    useEditorStore.setState({ openFile } as never);
    useTurnStore.setState({
      byId: new Map([["session-1", { ...emptySlice(), activePlanPath: ".litecode/plan/calm.md" }]]),
    });

    render(<ActivePlanChip sessionId="session-1" />);
    const chip = screen.getByRole("button", { name: "Open active plan" });
    // Mounted closed, opens on the next frame so the CSS transition runs.
    expect(chip.className).not.toContain("is-open");
    act(() => vi.advanceTimersByTime(20));
    expect(chip.className).toContain("is-open");

    fireEvent.click(chip);
    expect(openFile).toHaveBeenCalledWith(".litecode/plan/calm.md");
  });
});
