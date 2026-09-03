import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "../../../stores/settingsStore";
import { McpServersSection } from "./McpServersSection";

function openNew(scope: "Global" | "Workspace") {
  fireEvent.click(screen.getByRole("button", { name: "New MCP server" }));
  const panel = document.querySelector("[data-dropdown-panel]");
  expect(panel).toBeTruthy();
  fireEvent.click(within(panel as HTMLElement).getByRole("button", { name: scope }));
}

describe("McpServersSection persist UX", () => {
  const saveMcpServer = vi.fn(async () => undefined);
  const removeMcpServer = vi.fn(async () => undefined);

  beforeEach(() => {
    saveMcpServer.mockClear();
    removeMcpServer.mockClear();
    useSettingsStore.setState({
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      persistByDoc: {},
      saveMcpServer,
      removeMcpServer,
      startMcpServer: vi.fn(),
      restartMcpServer: vi.fn(),
      stopMcpServer: vi.fn(),
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not auto-save or show Fix fields to save when adding a server", async () => {
    vi.useFakeTimers();
    render(<McpServersSection />);
    openNew("Global");
    await vi.advanceTimersByTimeAsync(400);
    expect(screen.getByRole("button", { name: "Create" })).toBeTruthy();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
    expect(saveMcpServer).not.toHaveBeenCalled();
  });

  it("creates from the template JSON without auto-PUT while drafting", async () => {
    render(<McpServersSection />);
    openNew("Workspace");
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(saveMcpServer).toHaveBeenCalledWith(
        "filesystem",
        expect.objectContaining({ command: "npx" }),
        "workspace",
      );
    });
  });

  it("shows a form error instead of saving incomplete JSON", async () => {
    render(<McpServersSection />);
    openNew("Global");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "{}" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(
      await screen.findByText("Valid JSON with id and stdio command is required"),
    ).toBeTruthy();
    expect(saveMcpServer).not.toHaveBeenCalled();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
  });

  it("deletes an existing server", async () => {
    useSettingsStore.setState({
      mcpDefs: {
        global: [
          {
            id: "filesystem",
            command: "npx",
            args: ["-y", "server"],
            transport: { type: "stdio" },
            timeout: 60,
          },
        ],
        workspace: [],
      },
    });
    render(<McpServersSection />);
    fireEvent.click(screen.getByRole("button", { name: "Delete filesystem" }));
    await waitFor(() => {
      expect(removeMcpServer).toHaveBeenCalledWith("filesystem", "global");
    });
  });

  it("does not rewrite JSON when runtime becomes running", async () => {
    saveMcpServer.mockImplementation(async (id, def) => {
      useSettingsStore.setState({
        mcpDefs: {
          global: [{ id, ...def }],
          workspace: [],
        },
      });
    });
    render(<McpServersSection />);
    openNew("Global");
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(saveMcpServer).toHaveBeenCalled();
    });
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    const before = box.value;
    expect(before).toContain('"command": "npx"');
    expect(before).not.toContain("running");
    useSettingsStore.setState({
      mcpRuntime: {
        global: {
          filesystem: {
            status: "running",
            tools: [{ name: "echo", description: "" }],
          },
        },
        workspace: {},
      },
    });
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(before);
  });
});
