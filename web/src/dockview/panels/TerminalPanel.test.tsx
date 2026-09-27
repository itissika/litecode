import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IDockviewPanelProps } from "dockview-react";

// The real instance owns an xterm + a websocket pty; the sidebar contract is
// what this file checks.
vi.mock("./terminal/TerminalInstance", () => ({
  TerminalInstance: ({ tabKey }: { tabKey: string }) => (
    <div data-testid={`terminal-instance-${tabKey}`} />
  ),
}));

import { useTerminalTabs } from "../../lib/litecodeTerminal";
import { TerminalPanel } from "./TerminalPanel";

const WIDTH_KEY = "litecode-terminal-sidebar-width";

function panelProps(): IDockviewPanelProps {
  return {
    params: {},
    api: {
      group: {
        api: {
          isCollapsed: () => false,
          onDidCollapsedChange: () => ({ dispose: () => {} }),
        },
      },
    },
  } as unknown as IDockviewPanelProps;
}

beforeEach(() => {
  localStorage.clear();
  useTerminalTabs.setState({ tabs: [], activeKey: null });
});

afterEach(() => {
  cleanup();
  useTerminalTabs.setState({ tabs: [], activeKey: null });
});

describe("TerminalPanel sidebar", () => {
  it("names a tab by its shell until a command runs, and drops the index number", () => {
    useTerminalTabs.setState({
      tabs: [{ key: "t1", title: "Terminal 1", shell: "bash" }],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    expect(screen.getByText("bash")).toBeTruthy();
    expect(screen.queryByText("1")).toBeNull();
    // Terminal icon leads the row.
    expect(document.querySelector("aside svg")).toBeTruthy();
    expect(screen.getByTestId("terminal-instance-t1")).toBeTruthy();
  });

  it("swaps the shell for the last command summary once one ran", () => {
    useTerminalTabs.setState({
      tabs: [
        {
          key: "t1",
          title: "Terminal 1",
          shell: "powershell",
          lastCommand: "npm run dev",
        },
      ],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    expect(screen.getByText("npm run dev")).toBeTruthy();
    expect(screen.queryByText("powershell")).toBeNull();
  });

  it("reveals the trash only while the row is hovered", () => {
    useTerminalTabs.setState({
      tabs: [{ key: "t1", title: "Terminal 1", shell: "bash" }],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    const close = screen.getByRole("button", { name: "Close Terminal 1" });
    // Hidden (and untouchable) until the row is hovered, then the shared
    // group-hover pattern takes over — same one the session rows use.
    expect(close.className).toContain("opacity-0");
    expect(close.className).toContain("pointer-events-none");
    expect(close.className).toContain("group-hover:opacity-100");
    expect(close.className).toContain("group-hover:pointer-events-auto");
    expect(close.className).toContain("hover:text-(--_dk-ix-danger-fg-hover)");
    expect(close.closest(".group")).toBeTruthy();
  });

  it("spans the header divider edge to edge", () => {
    useTerminalTabs.setState({
      tabs: [{ key: "t1", title: "Terminal 1", shell: "bash" }],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    const divider = screen.getByTestId("terminal-header-divider");
    expect(divider.className).toContain("inset-x-0");
    // No side insets: the hairline is flush with both edges.
    expect(divider.getAttribute("style")).toBeNull();
    expect(divider.parentElement?.className).toContain("relative");
  });

  it("leaves rows colourless until hovered, marking the active one by brightness", () => {
    useTerminalTabs.setState({
      tabs: [
        { key: "t1", title: "Terminal 1", shell: "bash" },
        {
          key: "t2",
          title: "Terminal 2",
          shell: "bash",
          lastCommand: "npm test",
        },
      ],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    const active = screen.getByText("bash").parentElement as HTMLElement;
    const idle = screen.getByText("npm test").parentElement as HTMLElement;
    // No standing background: the hover state is the only fill, and it is the
    // shared file-tree token.
    const backgrounds = active.className
      .split(/\s+/)
      .filter((name) => name.includes("bg-("));
    expect(backgrounds).toEqual(["hover:bg-(--_dk-ix-bg-hover)"]);
    // The active terminal is told apart by a brighter icon + label instead.
    expect(active.className).toContain("text-(--_dk-text-primary)");
    expect(idle.className).toContain("text-(--_dk-text-secondary)");
    expect(active.className).not.toContain("opacity-");
    expect(active.className).not.toContain("surface-raised");
  });

  it("draws a visible hairline through the drag strip", () => {
    useTerminalTabs.setState({
      tabs: [{ key: "t1", title: "Terminal 1", shell: "bash" }],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    const handle = screen.getByTestId("terminal-sidebar-resize");
    expect(handle.className).toContain("after:bg-(--_dk-border-visible)");
    expect(handle.className).toContain("hover:bg-(--_dk-ix-bg-hover)");
  });

  it("right-aligns the new-terminal button", () => {
    useTerminalTabs.setState({
      tabs: [{ key: "t1", title: "Terminal 1", shell: "bash" }],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    const plus = screen.getByRole("button", { name: "New Terminal" });
    expect(plus.parentElement?.className).toContain("justify-end");
  });

  it("resizes the sidebar by dragging the seam, with a clamp", () => {
    useTerminalTabs.setState({
      tabs: [{ key: "t1", title: "Terminal 1", shell: "bash" }],
      activeKey: "t1",
    });

    render(<TerminalPanel {...panelProps()} />);

    const handle = screen.getByTestId("terminal-sidebar-resize");
    const sidebar = screen.getByTestId("terminal-sidebar");
    expect(sidebar.style.width).toBe("160px");

    // Dragging left widens it.
    fireEvent.pointerDown(handle, { pointerId: 1, clientX: 300 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 260 });
    expect(sidebar.style.width).toBe("200px");

    // Beyond the ceiling it stops at the cap.
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 0 });
    expect(sidebar.style.width).toBe("420px");

    fireEvent.pointerUp(handle, { pointerId: 1, clientX: 0 });
    expect(localStorage.getItem(WIDTH_KEY)).toBe("420");

    // The next mount comes back at the persisted width.
    cleanup();
    render(<TerminalPanel {...panelProps()} />);
    expect(screen.getByTestId("terminal-sidebar").style.width).toBe("420px");
  });
});
