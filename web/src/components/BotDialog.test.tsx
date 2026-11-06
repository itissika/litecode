import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BotInfo } from "../api/types";
import type { AgentProfile } from "../api/settings";
import { useBotStore } from "../stores/botStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { BotDialog } from "./BotDialog";

vi.mock("../api/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/settings")>();
  return {
    ...actual,
    getAgent: vi.fn(),
    getAvailableTools: vi.fn(),
    getMcpServers: vi.fn(),
  };
});

import { getAgent, getAvailableTools, getMcpServers } from "../api/settings";

function bot(patch: Partial<BotInfo> = {}): BotInfo {
  return {
    id: "b1",
    name: "Researcher",
    description: "finds papers",
    body: { system_prompt: "", tools: {}, allowed_subagents: [] },
    created_at: 1,
    session_id: "s1",
    position: { kind: "none" },
    sort_order: 1,
    ...patch,
  };
}

function profile(patch: Partial<AgentProfile> = {}): AgentProfile {
  return {
    role: "primary",
    model_ref: "main/default",
    system_prompt: "old prompt",
    description: "",
    tools: {},
    allowed_subagents: [],
    ...patch,
  };
}

beforeEach(() => {
  useBotStore.setState({ bots: [], botsLoading: false, botsError: null });
  useSessionStore.setState({
    primaryAgents: [{ id: "default", description: "General" }],
    activePrimary: "default",
    availableModels: [
      {
        id: "main/default",
        api_model_id: "default",
        provider_id: "main",
        label: "Default",
        context_window: 128000,
      },
    ],
    sessions: [],
  } as never);
  vi.mocked(getAgent).mockReset();
  vi.mocked(getAvailableTools).mockReset();
  vi.mocked(getMcpServers).mockReset();
  vi.mocked(getMcpServers).mockResolvedValue({ global: [], workspace: [] });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("BotDialog", () => {
  it("creates from a template body and reports the new bot", async () => {
    vi.mocked(getAgent).mockResolvedValue(profile());
    vi.mocked(getAvailableTools).mockResolvedValue([]);
    const created = bot({ id: "b-new", name: "New bot" });
    const sendRpc = vi.fn(async () => ({ bot: created }));
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onCreated = vi.fn();
    const onClose = vi.fn();

    render(<BotDialog mode={{ kind: "create" }} onClose={onClose} onCreated={onCreated} />);
    await screen.findByRole("dialog");
    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "New bot" },
    });
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "does things" },
    });
    await waitFor(() =>
      expect(screen.getByLabelText("System prompt")).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Create bot" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith("bot/create", {
        name: "New bot",
        description: "does things",
        body: {
          system_prompt: "old prompt",
          tools: {},
          allowed_subagents: [],
        },
      }),
    );
    expect(onCreated).toHaveBeenCalledWith(created);
    expect(onClose).toHaveBeenCalled();
  });

  it("loads and saves the bot body in edit mode", async () => {
    vi.mocked(getAvailableTools).mockResolvedValue([
      {
        id: "read",
        kind: "core",
        origin: "builtin",
        permission_surface: "fixed",
      },
    ]);
    const sendRpc = vi.fn(async (_method: string, params: unknown) => ({
      bot: { ...bot(), name: (params as { name: string }).name },
    }));
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onClose = vi.fn();

    render(<BotDialog mode={{ kind: "edit", bot: bot() }} onClose={onClose} />);
    await screen.findByRole("dialog");
    const prompt = await screen.findByLabelText("System prompt");
    // The bot editor reuses the Settings → Agents form styles and tool cards.
    expect(prompt.className).toContain("settings-input");
    expect(
      screen.getByRole("button", { name: /read tool binding, disabled/i }),
    ).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "Renamed" },
    });
    fireEvent.change(prompt, { target: { value: "new prompt" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith(
        "bot/update",
        expect.objectContaining({
          id: "b1",
          name: "Renamed",
          body: expect.objectContaining({
            system_prompt: "new prompt",
          }),
        }),
      ),
    );
    expect(onClose).toHaveBeenCalled();
  });
});