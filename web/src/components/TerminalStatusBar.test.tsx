import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useBashStore } from "../stores/bashStore";
import { TerminalStatusBar } from "./TerminalStatusBar";

afterEach(() => {
  cleanup();
  useBashStore.getState().reset();
});

describe("TerminalStatusBar", () => {
  it("renders nothing when there are no alive bash jobs", () => {
    const { container } = render(<TerminalStatusBar sessionId="s1" />);
    expect(container.firstChild).toBeNull();
  });

  it("shows a bouncing terminal icon and session job count", () => {
    useBashStore.getState().applySnapshot("s1", {
      jobs: [
        {
          id: "bg_a",
          call_id: "c1",
          command_preview: "sleep 1",
          output_file: ".litecode/bash/bg_a.output",
          started_at_ms: Date.now(),
        },
        {
          id: "bg_b",
          call_id: "c2",
          command_preview: "sleep 2",
          output_file: ".litecode/bash/bg_b.output",
          started_at_ms: Date.now(),
        },
      ],
      waits: [],
    });
    render(<TerminalStatusBar sessionId="s1" />);
    expect(screen.getByLabelText("2 active terminals")).toBeTruthy();
    expect(screen.getByText("×2")).toBeTruthy();
    expect(document.querySelector(".terminal-status-icon")).toBeTruthy();
  });

  it("cycles reveal to each job call_id on click", () => {
    const onRevealBash = vi.fn();
    useBashStore.getState().applySnapshot("s1", {
      jobs: [
        {
          id: "bg_a",
          call_id: "c1",
          command_preview: "sleep 1",
          output_file: ".litecode/bash/bg_a.output",
          started_at_ms: Date.now(),
        },
        {
          id: "bg_b",
          call_id: "c2",
          command_preview: "sleep 2",
          output_file: ".litecode/bash/bg_b.output",
          started_at_ms: Date.now(),
        },
      ],
      waits: [],
    });
    render(<TerminalStatusBar sessionId="s1" onRevealBash={onRevealBash} />);
    const chip = screen.getByRole("button", { name: "2 active terminals" });
    fireEvent.click(chip);
    fireEvent.click(chip);
    fireEvent.click(chip);
    expect(onRevealBash.mock.calls.map((c) => c[0])).toEqual(["c1", "c2", "c1"]);
  });
});
