import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSubagentStore } from "../stores/subagentStore";
import { SubagentStatusBar } from "./SubagentStatusBar";

const job = {
  id: "sa_a",
  call_id: "c1",
  agent_name: "researcher",
  prompt_preview: "find the wiring",
  started_at_ms: Date.now(),
};

const twoJobs = [
  job,
  {
    id: "sa_b",
    call_id: "c2",
    agent_name: "writer",
    prompt_preview: "draft the summary",
    started_at_ms: Date.now(),
  },
];

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  useSubagentStore.getState().reset();
});

describe("SubagentStatusBar", () => {
  it("renders nothing when there are no running workers", () => {
    const { container } = render(<SubagentStatusBar sessionId="s1" />);
    expect(container.firstChild).toBeNull();
  });

  it("shows a bouncing icon and session worker count after the 1s debounce", () => {
    vi.useFakeTimers();
    useSubagentStore.getState().applySnapshot("s1", { jobs: twoJobs, waits: [] });
    render(<SubagentStatusBar sessionId="s1" />);
    // Not yet — the worker must run continuously for 1s before surfacing.
    expect(screen.queryByLabelText(/running subagent/)).toBeNull();
    act(() => vi.advanceTimersByTime(1000));
    const chip = screen.getByLabelText("2 running subagents");
    expect(chip.tagName).toBe("DIV");
    expect(screen.getByText("×2")).toBeTruthy();
    expect(document.querySelector(".subagent-status-icon")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("never shows the chip for a worker that ends before the 1s debounce", () => {
    vi.useFakeTimers();
    render(<SubagentStatusBar sessionId="s1" />);

    act(() => {
      useSubagentStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    // Worker ends after 50ms — the debounce never fires, so no chip at all.
    act(() => vi.advanceTimersByTime(50));
    act(() => {
      useSubagentStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    act(() => vi.advanceTimersByTime(2000));
    expect(screen.queryByLabelText(/running subagent/)).toBeNull();
  });

  it("holds the chip 1s even after a long-running worker ends", () => {
    vi.useFakeTimers();
    render(<SubagentStatusBar sessionId="s1" />);

    act(() => {
      useSubagentStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByLabelText("1 running subagent")).toBeTruthy();

    // Worker runs 10s total, then ends — the hold applies the same as ever.
    act(() => vi.advanceTimersByTime(9000));
    act(() => {
      useSubagentStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    expect(screen.getByLabelText("0 running subagents")).toBeTruthy();

    // Still inside the 1s hold.
    act(() => vi.advanceTimersByTime(600));
    expect(screen.getByLabelText("0 running subagents")).toBeTruthy();

    // Hold expires, exit animation runs before unmount.
    act(() => vi.advanceTimersByTime(600));
    act(() => vi.advanceTimersByTime(300));
    expect(screen.queryByLabelText(/running subagent/)).toBeNull();
  });

  it("animates out before unmount", () => {
    vi.useFakeTimers();
    render(<SubagentStatusBar sessionId="s1" />);

    act(() => {
      useSubagentStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    const chip = screen.getByLabelText("1 running subagent");
    // Mounted closed, opens on the next frame so the CSS transition runs.
    expect(chip.className).not.toContain("is-open");
    act(() => vi.advanceTimersByTime(20));
    expect(chip.className).toContain("is-open");

    // Worker ends → hold expires → chip retracts (is-open dropped) before unmount.
    act(() => {
      useSubagentStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    expect(chip.className).not.toContain("is-open");

    act(() => vi.advanceTimersByTime(300));
    expect(screen.queryByLabelText(/running subagent/)).toBeNull();
  });
});
