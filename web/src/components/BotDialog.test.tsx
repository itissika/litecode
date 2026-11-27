import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BotBody, BotInfo } from "../api/types";
import type {
  AgentProfile,
  AgentToolBinding,
  AvailableTool,
} from "../api/settings";
import { useBotStore } from "../stores/botStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { botDialogSize, composeBotDraftTools, BotDialog } from "./BotDialog";

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

/** The locked `default` spec the backend sends for `getAgent("default")`. */
function defaultAgent(patch: Partial<AgentProfile> = {}): AgentProfile {
  return {
    role: "primary",
    model_ref: "main/default",
    system_prompt: "builtin:default",
    description: "General purpose agent",
    tools: {
      read: { enabled: true, preset: "ALL" },
      exec: { enabled: true, preset: "SAFE" },
      code_search: { enabled: true, preset: null },
      webfetch: { enabled: true, preset: null },
      websearch: { enabled: true, preset: null },
    },
    allowed_subagents: ["explore"],
    builtin: true,
    ...patch,
  };
}

/** Backend catalog: core cards are locked, the rest are user-configurable. */
const toolCatalog: AvailableTool[] = [
  {
    id: "read",
    kind: "core",
    origin: "builtin",
    permission_surface: "preset",
    locked: true,
  },
  {
    id: "exec",
    kind: "core",
    origin: "builtin",
    permission_surface: "preset",
    locked: true,
  },
  {
    id: "code_search",
    kind: "engine",
    origin: "builtin",
    permission_surface: "fixed",
    locked: false,
  },
  {
    id: "webfetch",
    kind: "engine",
    origin: "builtin",
    permission_surface: "fixed",
    locked: false,
  },
  {
    id: "websearch",
    kind: "engine",
    origin: "builtin",
    permission_surface: "fixed",
    locked: false,
  },
  {
    id: "lsp",
    kind: "engine",
    origin: "workspace",
    permission_surface: "fixed",
    locked: false,
  },
];

const ROLE_LABEL = "Role & responsibilities";

function toolCard(toolId: string): HTMLElement {
  const card = toolToggle(toolId).closest(".tool-binding-card");
  expect(card).toBeTruthy();
  return card as HTMLElement;
}

function toolToggle(toolId: string): HTMLButtonElement {
  return screen.getByRole("button", {
    name: new RegExp(`^${toolId} tool binding`, "i"),
  }) as HTMLButtonElement;
}

function expectCard(toolId: string, state: { enabled: boolean; locked?: boolean }) {
  const card = toolCard(toolId);
  expect(card.getAttribute("data-enabled")).toBe(state.enabled ? "true" : "false");
  expect(card.getAttribute("data-locked")).toBe(
    state.locked ? "true" : null,
  );
  // Locked cards drop the on/off tag: they are records, not controls.
  if (!state.locked) {
    expect(within(card).getByText(state.enabled ? "On" : "Off")).toBeTruthy();
  }
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
      {
        id: "main/alpha",
        api_model_id: "alpha",
        provider_id: "main",
        label: "Alpha",
        context_window: 128000,
      },
    ],
    sessions: [],
  } as never);
  vi.mocked(getAgent).mockReset();
  vi.mocked(getAvailableTools).mockReset();
  vi.mocked(getMcpServers).mockReset();
  vi.mocked(getAgent).mockResolvedValue(defaultAgent());
  vi.mocked(getAvailableTools).mockResolvedValue(toolCatalog);
  vi.mocked(getMcpServers).mockResolvedValue({ global: [], workspace: [] });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("BotDialog sheet resize", () => {
  it("grows by twice the drag delta so the corner tracks the pointer", () => {
    expect(
      botDialogSize({ w: 800, h: 500 }, 100, 50, { w: 1600, h: 1000 }),
    ).toEqual({ w: 1000, h: 600 });
  });

  it("clamps to the resting bounds of the sheet", () => {
    expect(
      botDialogSize({ w: 800, h: 500 }, -4000, -4000, { w: 1600, h: 1000 }),
    ).toEqual({ w: 520, h: 360 });
    expect(
      botDialogSize({ w: 800, h: 500 }, 4000, 4000, { w: 2000, h: 1000 }),
    ).toEqual({ w: 1120, h: 850 });
  });

  it("renders the grip on a wider sheet", async () => {
    render(<BotDialog mode={{ kind: "edit", bot: bot() }} onClose={vi.fn()} />);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.querySelector("[data-resize-grip]")).toBeTruthy();
    expect(dialog.className).toContain("max-w-[70rem]");
  });
});

describe("BotDialog create", () => {
  it("has no template picker and starts the role box empty", async () => {
    const onClose = vi.fn();
    render(<BotDialog mode={{ kind: "create" }} onClose={onClose} />);
    await screen.findByRole("dialog");

    // D2: the base is always the built-in `default`; nothing to choose.
    expect(vi.mocked(getAgent).mock.calls).toEqual([["default"]]);
    expect(screen.queryByText("Template agent")).toBeNull();
    expect(screen.queryByText(/available as a template/i)).toBeNull();

    const roleBox = (await screen.findByLabelText(
      ROLE_LABEL,
    )) as HTMLTextAreaElement;
    // The `builtin:default` marker is never copied into the user's box.
    expect(roleBox.value).toBe("");
    expect(roleBox.placeholder).toMatch(/release captain/i);
  });

  it("shows the locked core cards and submits only non-core overrides", async () => {
    const created = bot({ id: "b-new", name: "Release bot", session_id: "s-new" });
    const sendRpc = vi.fn(async (_method: string, _params?: unknown) => ({
      bot: created,
    }));
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onCreated = vi.fn();
    const onClose = vi.fn();

    render(
      <BotDialog
        mode={{ kind: "create" }}
        onClose={onClose}
        onCreated={onCreated}
      />,
    );
    await screen.findByRole("dialog");
    const roleBox = await screen.findByLabelText(ROLE_LABEL);

    // Locked core cards mirror the `default` spec and cannot be touched.
    expectCard("read", { enabled: true, locked: true });
    expectCard("exec", { enabled: true, locked: true });
    expect(toolToggle("read").disabled).toBe(true);
    expect(toolToggle("read").getAttribute("aria-disabled")).toBe("true");
    // No lock tag on the card itself: the read-only styling carries that.
    expect(within(toolCard("read")).queryByText("Locked")).toBeNull();
    // The spec's optional defaults start on (D3) and stay user-configurable.
    expectCard("code_search", { enabled: true });
    expectCard("webfetch", { enabled: true });
    expectCard("websearch", { enabled: true });
    expectCard("lsp", { enabled: false });

    fireEvent.click(toolToggle("read"));
    expectCard("read", { enabled: true, locked: true });

    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "Release bot" },
    });
    fireEvent.change(roleBox, { target: { value: "Release captain" } });
    fireEvent.click(toolToggle("webfetch"));
    expectCard("webfetch", { enabled: false });
    fireEvent.click(toolToggle("lsp"));
    expectCard("lsp", { enabled: true });

    fireEvent.click(screen.getByRole("button", { name: "Create bot" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith(
        "bot/create",
        expect.objectContaining({ name: "Release bot" }),
      ),
    );
    const params = sendRpc.mock.calls[0][1] as {
      name: string;
      description: string;
      body: BotBody;
    };
    expect(sendRpc.mock.calls.map(([method]) => method)).toEqual(["bot/create"]);
    // The role text travels verbatim; the backend wraps it as `# Bot Role`.
    expect(params.body.system_prompt).toBe("Release captain");
    // Fixed by the `default` spec, never sent by the record.
    expect(params.body.allowed_subagents).toEqual([]);
    // Core keys are spec-owned: the payload stays clean (I1/I3).
    expect(Object.keys(params.body.tools)).not.toContain("read");
    expect(Object.keys(params.body.tools)).not.toContain("exec");
    expect(params.body.tools.webfetch).toEqual({
      enabled: false,
      preset: null,
    });
    expect(params.body.tools.lsp.enabled).toBe(true);
    expect(params.body.tools.code_search.enabled).toBe(true);
    expect(params.body.tools.websearch.enabled).toBe(true);
    expect(onCreated).toHaveBeenCalledWith(created);
    expect(onClose).toHaveBeenCalled();
  });
});

describe("BotDialog edit", () => {
  const releaseBody: BotBody = {
    system_prompt: "Release captain",
    tools: {
      webfetch: { enabled: false, preset: null },
      lsp: { enabled: true, preset: null },
    },
    allowed_subagents: [],
  };

  it("rebuilds the grid from the default spec plus the saved overrides", async () => {
    render(
      <BotDialog
        mode={{ kind: "edit", bot: bot({ body: releaseBody }) }}
        onClose={vi.fn()}
      />,
    );
    await screen.findByRole("dialog");
    const roleBox = (await screen.findByLabelText(
      ROLE_LABEL,
    )) as HTMLTextAreaElement;
    expect(roleBox.value).toBe("Release captain");

    // Locked core cards come from `getAgent("default")`, not from the record.
    expectCard("read", { enabled: true, locked: true });
    expectCard("exec", { enabled: true, locked: true });
    // The record's non-core intent is restored on top of the spec defaults.
    expectCard("webfetch", { enabled: false });
    expectCard("code_search", { enabled: true });
    expectCard("websearch", { enabled: true });
    expectCard("lsp", { enabled: true });
  });

  it("saves the role text and keeps core keys out of the payload", async () => {
    const sendRpc = vi.fn(async (_method: string, params: unknown) => ({
      bot: { ...bot(), name: (params as { name: string }).name },
    }));
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onClose = vi.fn();

    render(
      <BotDialog
        mode={{ kind: "edit", bot: bot({ body: releaseBody }) }}
        onClose={onClose}
      />,
    );
    await screen.findByRole("dialog");
    const roleBox = await screen.findByLabelText(ROLE_LABEL);
    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "Renamed" },
    });
    fireEvent.change(roleBox, { target: { value: "Release captain, v2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith(
        "bot/update",
        expect.objectContaining({ id: "b1", name: "Renamed" }),
      ),
    );
    const params = sendRpc.mock.calls[0][1] as { body: BotBody };
    expect(params.body.system_prompt).toBe("Release captain, v2");
    expect(params.body.allowed_subagents).toEqual([]);
    expect(Object.keys(params.body.tools)).not.toContain("read");
    expect(Object.keys(params.body.tools)).not.toContain("exec");
    expect(params.body.tools.webfetch).toEqual({ enabled: false, preset: null });
    expect(params.body.tools.lsp.enabled).toBe(true);
    expect(sendRpc.mock.calls.map(([method]) => method)).toEqual(["bot/update"]);
    expect(onClose).toHaveBeenCalled();
  });

  it("stages the model and applies it to the bot session on save", async () => {
    useSessionStore.setState({
      sessions: [{ id: "s1", model_id: "main/default" }],
    } as never);
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "bot/update") return { bot: bot({ name: "Renamed" }) };
      return {};
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onClose = vi.fn();

    render(<BotDialog mode={{ kind: "edit", bot: bot() }} onClose={onClose} />);
    await screen.findByRole("dialog");
    await screen.findByLabelText(ROLE_LABEL);
    fireEvent.click(screen.getByRole("button", { name: "Default" }));
    fireEvent.click(screen.getByRole("button", { name: "Alpha" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith("agent/set-model", {
        session_id: "s1",
        model_id: "main/alpha",
      }),
    );
    expect(sendRpc.mock.calls.map(([method]) => method)).toEqual([
      "bot/update",
      "agent/set-model",
    ]);
    expect(onClose).toHaveBeenCalled();
  });

  it("applies a create-time model to the new bot session", async () => {
    const created = bot({ id: "b-new", name: "New bot", session_id: "s-new" });
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "bot/create") return { bot: created };
      return {};
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onCreated = vi.fn();
    const onClose = vi.fn();

    render(
      <BotDialog
        mode={{ kind: "create" }}
        onClose={onClose}
        onCreated={onCreated}
      />,
    );
    await screen.findByRole("dialog");
    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "New bot" },
    });
    await screen.findByLabelText(ROLE_LABEL);
    fireEvent.click(screen.getByRole("button", { name: "Select model" }));
    fireEvent.click(screen.getByRole("button", { name: "Alpha" }));
    fireEvent.click(screen.getByRole("button", { name: "Create bot" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith("agent/set-model", {
        session_id: "s-new",
        model_id: "main/alpha",
      }),
    );
    expect(sendRpc.mock.calls.map(([method]) => method)).toEqual([
      "bot/create",
      "agent/set-model",
    ]);
    expect(onCreated).toHaveBeenCalledWith(created);
    expect(onClose).toHaveBeenCalled();
  });

  it("disables the model picker while the bot is running", async () => {
    useSessionStore.setState({
      sessions: [{ id: "s1", model_id: "main/default", running: true }],
    } as never);

    render(<BotDialog mode={{ kind: "edit", bot: bot() }} onClose={vi.fn()} />);
    await screen.findByRole("dialog");
    await screen.findByLabelText(ROLE_LABEL);
    expect(
      (screen.getByRole("button", { name: "Default" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("locks the role box and tool grid while busy, but still renames", async () => {
    useSessionStore.setState({
      sessions: [{ id: "s1", model_id: "main/default", running: true }],
    } as never);
    const sendRpc = vi.fn(async (method: string, _params?: unknown) => {
      if (method === "bot/update") return { bot: bot({ name: "Renamed" }) };
      return {};
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    const onClose = vi.fn();

    render(
      <BotDialog
        mode={{ kind: "edit", bot: bot({ body: releaseBody }) }}
        onClose={onClose}
      />,
    );
    await screen.findByRole("dialog");
    const roleBox = await screen.findByLabelText(ROLE_LABEL);
    expect(roleBox).toHaveProperty("disabled", true);
    expect(toolToggle("read")).toHaveProperty("disabled", true);
    expect(toolToggle("webfetch")).toHaveProperty("disabled", true);
    expect(toolToggle("lsp")).toHaveProperty("disabled", true);

    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "Renamed" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(sendRpc).toHaveBeenCalledWith(
        "bot/update",
        expect.objectContaining({ id: "b1", name: "Renamed" }),
      ),
    );
    const params = sendRpc.mock.calls[0][1] as Record<string, unknown>;
    // A running bot keeps its body: the identity update carries no `body`.
    expect("body" in params).toBe(false);
    expect(onClose).toHaveBeenCalled();
  });

  it("lets Escape close the model menu without closing the dialog", async () => {
    useSessionStore.setState({
      sessions: [{ id: "s1", model_id: "main/default" }],
    } as never);

    render(<BotDialog mode={{ kind: "edit", bot: bot() }} onClose={vi.fn()} />);
    await screen.findByRole("dialog");
    await screen.findByLabelText(ROLE_LABEL);
    const trigger = screen.getByRole("button", { name: "Default" });
    fireEvent.click(trigger);
    expect(document.querySelector("[data-dropdown-panel]")).toBeTruthy();

    fireEvent.keyDown(trigger, { key: "Escape" });

    expect(screen.getByRole("dialog")).toBeTruthy();
    await waitFor(() =>
      expect(document.querySelector("[data-dropdown-panel]")).toBeNull(),
    );
  });
});

describe("BotDialog load failure", () => {
  it("reports a base-spec failure and blocks submit", async () => {
    vi.mocked(getAgent).mockRejectedValue(new Error("agents unavailable"));
    const sendRpc = vi.fn();
    useConnectionStore.setState({ state: "connected", sendRpc } as never);

    render(<BotDialog mode={{ kind: "create" }} onClose={vi.fn()} />);
    await screen.findByRole("dialog");
    const notices = await screen.findAllByText(/Could not load the bot profile/i);
    expect(notices.length).toBeGreaterThan(0);
    expect(screen.getByText(/agents unavailable/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Bot name"), {
      target: { value: "Blocked bot" },
    });
    const submit = screen.getByRole("button", {
      name: "Create bot",
    }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.click(submit);
    expect(sendRpc).not.toHaveBeenCalled();
  });
});

describe("composeBotDraftTools", () => {
  it("mirrors locked spec cards and layers only non-core overrides", () => {
    const specTools: Record<string, AgentToolBinding> = {
      read: { enabled: true, preset: "SAFE" },
    };
    const draft = composeBotDraftTools(
      specTools,
      [
        {
          id: "read",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          locked: true,
        },
      ],
      {
        // A stale core row in the record must not move the locked card.
        read: { enabled: false, preset: "ALL" },
        webfetch: { enabled: false, preset: null },
      },
    );
    expect(draft.read).toEqual({ enabled: true, preset: "SAFE" });
    expect(draft.webfetch).toEqual({ enabled: false, preset: null });
    // Spec optional defaults start on with no dial.
    expect(draft.code_search).toEqual({ enabled: true, preset: null });
    expect(draft.websearch).toEqual({ enabled: true, preset: null });
  });
});
