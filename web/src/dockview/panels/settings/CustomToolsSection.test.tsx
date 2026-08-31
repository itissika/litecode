import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CustomToolDefinition } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { CustomToolsSection } from "./CustomToolsSection";

const echoTool: CustomToolDefinition = {
  name: "echo_py",
  description: "Echo",
  command: "python",
  args: ["echo.py"],
  timeout: 120,
  schema: { type: "object", properties: {}, required: [] },
};

function openNew(scope: "Global" | "Workspace") {
  fireEvent.click(screen.getByRole("button", { name: "New custom tool" }));
  const panel = document.querySelector("[data-dropdown-panel]");
  expect(panel).toBeTruthy();
  fireEvent.click(within(panel as HTMLElement).getByRole("button", { name: scope }));
}

describe("CustomToolsSection persist UX", () => {
  const saveCustomTool = vi.fn(async () => undefined);
  const removeCustomTool = vi.fn(async () => undefined);

  beforeEach(() => {
    saveCustomTool.mockClear();
    removeCustomTool.mockClear();
    useSettingsStore.setState({
      customTools: { global: [], workspace: [] },
      persistStatus: "idle",
      saveCustomTool,
      removeCustomTool,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not auto-save or show Fix fields to save when adding a tool", async () => {
    vi.useFakeTimers();
    render(<CustomToolsSection />);
    openNew("Global");
    await vi.advanceTimersByTimeAsync(400);
    expect(screen.getByRole("button", { name: "Create" })).toBeTruthy();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
    expect(saveCustomTool).not.toHaveBeenCalled();
  });

  it("creates from the template JSON without auto-PUT while drafting", async () => {
    render(<CustomToolsSection />);
    openNew("Workspace");
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(saveCustomTool).toHaveBeenCalledWith(
        "echo_py",
        expect.objectContaining({ name: "echo_py", command: "python" }),
        "workspace",
      );
    });
  });

  it("shows a form error instead of saving incomplete JSON", async () => {
    render(<CustomToolsSection />);
    openNew("Global");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "{}" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(
      await screen.findByText("Valid JSON with name, command, and schema is required"),
    ).toBeTruthy();
    expect(saveCustomTool).not.toHaveBeenCalled();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
  });

  it("deletes an existing tool", async () => {
    useSettingsStore.setState({
      customTools: { global: [echoTool], workspace: [] },
    });
    render(<CustomToolsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Delete echo_py" }));
    await waitFor(() => {
      expect(removeCustomTool).toHaveBeenCalledWith("echo_py", "global");
    });
  });
});
