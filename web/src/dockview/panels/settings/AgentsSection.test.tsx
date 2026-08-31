import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentProfile, ModelDefinition } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { AgentsSection } from "./AgentsSection";

const model: ModelDefinition = {
  id: "m1",
  adapter_id: "openai",
  provider_ref: "prov",
  label: "GPT",
  config: {
    api_model_id: "gpt-4o",
    context_window: 200_000,
    max_tokens: 8192,
    capabilities: ["text"],
  },
};

function profile(patch: Partial<AgentProfile> = {}): AgentProfile {
  return {
    role: "primary",
    model_ref: "m1",
    system_prompt: "",
    temperature: 0.7,
    max_steps: 50,
    description: "",
    tools: {},
    allowed_subagents: [],
    ...patch,
  };
}

describe("AgentsSection persist UX", () => {
  const saveAgent = vi.fn(async () => undefined);
  const createAgent = vi.fn(async () => undefined);
  const removeAgent = vi.fn(async () => undefined);
  const refreshAgents = vi.fn(async () => undefined);

  beforeEach(() => {
    saveAgent.mockClear();
    createAgent.mockClear();
    removeAgent.mockClear();
    refreshAgents.mockClear();
    useSettingsStore.setState({
      models: { m1: model },
      availableTools: [],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: { default: profile() },
      persistStatus: "idle",
      saveAgent,
      createAgent,
      removeAgent,
      refreshAgents,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("does not auto-save or show Fix fields to save when adding an agent", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    await vi.advanceTimersByTimeAsync(400);
    expect(screen.getByPlaceholderText("my_agent")).toBeTruthy();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
    expect(saveAgent).not.toHaveBeenCalled();
    expect(createAgent).not.toHaveBeenCalled();
  });

  it("creates once the new agent has an id", async () => {
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    fireEvent.change(screen.getByPlaceholderText("my_agent"), {
      target: { value: "my_agent" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(createAgent).toHaveBeenCalledWith("my_agent", expect.any(Object));
    });
    expect(saveAgent).not.toHaveBeenCalled();
  });

  it("PUTs the selected agent after an edit", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    const description = screen.getAllByRole("textbox").find(
      (el) => (el as HTMLInputElement).type !== "number" && (el as HTMLTextAreaElement).rows == null,
    );
    expect(description).toBeTruthy();
    fireEvent.change(description!, { target: { value: "Helper" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledWith(
      "default",
      expect.objectContaining({ description: "Helper" }),
    );
  });

  it("deletes a non-protected agent after confirm", async () => {
    vi.stubGlobal("confirm", vi.fn(() => true));
    useSettingsStore.setState({
      agentIds: ["helper"],
      selectedAgentId: "helper",
      agents: { helper: profile({ role: "subagent" }) },
    });
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => {
      expect(removeAgent).toHaveBeenCalledWith("helper");
    });
  });
});
