import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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

function startNew(scope: "Global" | "Workspace", kind: "tool" | "suite") {
  fireEvent.click(screen.getByRole("button", { name: "New" }));
  const panel = document.querySelector("[data-dropdown-panel]");
  expect(panel).toBeTruthy();
  const name = `New ${scope.toLowerCase()} ${kind}`;
  fireEvent.click(within(panel as HTMLElement).getByRole("button", { name }));
}

describe("CustomToolsSection persist UX", () => {
  const saveCustomTool = vi.fn(async () => undefined);
  const removeCustomTool = vi.fn(async () => undefined);
  const saveCustomSuite = vi.fn(async () => undefined);

  beforeEach(() => {
    saveCustomTool.mockClear();
    removeCustomTool.mockClear();
    saveCustomSuite.mockClear();
    useSettingsStore.setState({
      customTools: { global: [], workspace: [], suites: { global: {}, workspace: {} }, diagnostics: [], code_rejections: [] },
      persistByDoc: {},
      saveCustomTool,
      removeCustomTool,
      saveCustomSuite,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not auto-save or show Fix fields to save when adding a tool", async () => {
    render(<CustomToolsSection />);
    startNew("Global", "tool");
    expect(await screen.findByRole("button", { name: "Create" })).toBeTruthy();
    await new Promise((resolve) => setTimeout(resolve, 450));
    expect(screen.queryByText("Fix fields to save")).toBeNull();
    expect(saveCustomTool).not.toHaveBeenCalled();
  });

  it("creates from the template JSON without auto-PUT while drafting", async () => {
    render(<CustomToolsSection />);
    startNew("Workspace", "tool");
    fireEvent.click(await screen.findByRole("button", { name: "Create" }));
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
    startNew("Global", "tool");
    const textarea = await screen.findByRole("textbox");
    fireEvent.change(textarea, { target: { value: "{}" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(
      await screen.findByText(
        "Valid JSON with name, command, and schema is required",
      ),
    ).toBeTruthy();
    expect(saveCustomTool).not.toHaveBeenCalled();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
  });

  it("deletes an existing tool", async () => {
    useSettingsStore.setState({
      customTools: { global: [echoTool], workspace: [], suites: { global: {}, workspace: {} }, diagnostics: [], code_rejections: [] },
    });
    render(<CustomToolsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Delete echo_py" }));
    await waitFor(() => {
      expect(removeCustomTool).toHaveBeenCalledWith("echo_py", "global");
    });
  });

  it("shows code admission warnings instead of hiding rejected tools", () => {
    useSettingsStore.setState({
      customTools: {
        global: [echoTool],
        workspace: [],
        suites: { global: {}, workspace: {} },
        diagnostics: [],
        code_rejections: [
          { name: "deck.slide", reason: "tool 'deck.slide' has no data schema" },
        ],
      },
    });
    render(<CustomToolsSection />);
    expect(screen.getByText("Code admission warnings")).toBeTruthy();
    expect(screen.getByText(/has no data schema/)).toBeTruthy();
  });

  it("keeps creation in the header, leaving the body to the two scope cards", () => {
    render(<CustomToolsSection />);
    expect(screen.getAllByRole("button", { name: "New" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "New global" })).toBeNull();
    expect(screen.queryByRole("button", { name: "New workspace" })).toBeNull();
    expect(screen.getByText("Global")).toBeTruthy();
    expect(screen.getByText("Workspace")).toBeTruthy();
  });

  it("creates a suite as one group definition", async () => {
    render(<CustomToolsSection />);
    startNew("Global", "suite");
    fireEvent.change(await screen.findByPlaceholderText("suite_id"), {
      target: { value: "deck" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save suite" }));
    await waitFor(() => {
      expect(saveCustomSuite).toHaveBeenCalledWith(
        "deck",
        { surface: "code", label: "Deck", rules: [] },
        "global",
      );
    });
  });
});
