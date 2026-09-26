import { cleanup, act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { uploadMedia } from "../api/media";
import { useConnectionStore } from "../stores/connectionStore";
import { appendComposerText } from "../stores/composerDraft";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";
import { AgentChatInput } from "./AgentChatInput";

vi.mock("../lib/imageNormalize", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/imageNormalize")>();
  return {
    ...actual,
    normalizeImage: vi.fn(async (file: Blob) => file),
  };
});

vi.mock("../api/media", () => ({
  uploadMedia: vi.fn(),
}));

const MEDIA_REF = `litecode-media:${"a".repeat(64)}.jpg`;

function seedSession(sessionId: string) {
  useSessionStore.setState({
    primaryAgents: [],
    availableModels: [
      {
        id: "openai/model-1",
        api_model_id: "model-1",
        provider_id: "openai",
        label: "Model 1",
        context_window: 1000,
      },
    ],
    byId: new Map([
      [
        sessionId,
        {
          activePrimary: "default",
          pendingPrimaryId: null,
          modelId: "openai/model-1",
          apiModelId: "model-1",
          label: "Model 1",
          thinkingTier: "medium",
          contextMode: "standard",
          pendingThinkingTier: null,
          pendingContextMode: null,
          maxFileRevertK: null,
        },
      ],
    ]),
  } as never);
}

beforeEach(() => {
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useToastStore.setState({ toasts: [] });
  useConnectionStore.setState({
    state: "connected",
    sendRpc: vi.fn(async () => ({ started: true })),
  } as never);
  seedSession("session-1");
});

afterEach(() => {
  cleanup();
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useToastStore.setState({ toasts: [] });
  useSessionStore.setState({
    byId: new Map(),
    primaryAgents: [],
    availableModels: [],
  } as never);
});

describe("AgentChatInput silent send", () => {
  it("queues on Enter while a turn runs: no toast, no optimistic row, draft clears on ack", async () => {
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "session/pending-enqueue") {
        return {
          queued: true,
          pending_messages: [{ id: "p1", text: "second try" }],
        };
      }
      return { started: true };
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    useTurnStore.setState({
      byId: new Map([["session-1", { ...EMPTY_SLICE, runState: "running" }]]),
    });
    render(<AgentChatInput sessionId="session-1" />);

    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "second try" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(sendRpc).toHaveBeenCalledWith("session/pending-enqueue", {
      text: "second try",
      session_id: "session-1",
    });
    // The queue lives on the server: no optimistic transcript row, no toast.
    expect(
      useMessageStore.getState().bySession.get("session-1")?.pendingUser,
    ).toBeFalsy();
    expect(useToastStore.getState().toasts).toEqual([]);
    await waitFor(() =>
      expect((textarea as HTMLTextAreaElement).value).toBe(""),
    );
    expect(
      useTurnStore.getState().byId.get("session-1")?.pendingMessages,
    ).toEqual([{ id: "p1", text: "second try" }]);
  });

  it("swaps the single live-turn button to queue when a draft exists", () => {
    useTurnStore.setState({
      byId: new Map([["session-1", { ...EMPTY_SLICE, runState: "running" }]]),
    });
    render(<AgentChatInput sessionId="session-1" />);

    // No draft: the one action button is Cancel, glowing with the turn.
    const cancel = screen.getByTitle("Cancel");
    expect(cancel.className).toContain("send-spin-glow");
    expect(screen.queryByTitle("Queue for the next step")).toBeNull();
    expect(screen.queryByTitle("Send")).toBeNull();

    // A draft turns the same slot into the queue action, glow kept.
    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "steer" } });
    const queue = screen.getByTitle("Queue for the next step");
    expect(queue.className).toContain("send-spin-glow");
    expect(screen.queryByTitle("Cancel")).toBeNull();
    expect(
      screen.getAllByTitle(/^(Cancel|Queue for the next step|Send)$/),
    ).toHaveLength(1);
  });

  it("queues while an auto compaction runs in the turn; a standalone one stays blocked", async () => {
    const sendRpc = vi.fn(async () => ({ queued: true }));
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    // Auto compaction: the turn is alive, so the queue action stays available.
    useTurnStore.setState({
      byId: new Map([
        ["session-1", { ...EMPTY_SLICE, runState: "running", compacting: true }],
      ]),
    });
    render(<AgentChatInput sessionId="session-1" />);

    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "steer" } });
    expect(screen.getByTitle("Queue for the next step")).toBeTruthy();
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });
    expect(sendRpc).toHaveBeenCalledWith("session/pending-enqueue", {
      text: "steer",
      session_id: "session-1",
    });

    // Standalone compaction on an idle session is still blocked: no send, no
    // queue, and Enter stays silent (the exclusive lease owns the session).
    act(() => {
      useTurnStore.setState({
        byId: new Map([
          ["session-1", { ...EMPTY_SLICE, runState: "idle", compacting: true }],
        ]),
      });
    });
    fireEvent.change(textarea, { target: { value: "steer again" } });
    const send = screen.getByTitle("Send") as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    expect(screen.queryByTitle("Queue for the next step")).toBeNull();
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });
    expect(sendRpc).not.toHaveBeenCalledWith("agent/run", expect.anything());
  });

  it("stacks the three glyphs in one box and scales the live one in", () => {
    render(<AgentChatInput sessionId="session-1" />);
    // Only the live layer is on; the others are held at scale .35 / alpha 0 so
    // they can cross-scale into place when the state flips. The cancel skin nests
    // a box of its own (stop square ↔ cancelling swirl), read separately.
    const onFlags = (box: Element | null) =>
      Array.from(box!.children).map((n) => n.getAttribute("data-on"));
    const layersOf = (btn: HTMLElement) =>
      onFlags(btn.querySelector(".composer-action-icons"));
    const innerOf = (btn: HTMLElement) =>
      onFlags(
        btn.querySelector(".composer-action-icons .composer-action-icons"),
      );

    const send = screen.getByTitle("Send");
    expect(send.getAttribute("type")).toBe("submit");
    expect(layersOf(send)).toEqual(["true", "false", "false"]);
    // The queue glyph is that same arrow turned 90° counter-clockwise (up).
    expect(
      send.querySelector(".composer-action-icons")!.children[1]!.getAttribute(
        "class",
      ),
    ).toContain("-rotate-90");

    act(() => {
      useTurnStore.setState({
        byId: new Map([
          ["session-1", { ...EMPTY_SLICE, runState: "running" }],
        ]),
      });
    });
    const cancel = screen.getByTitle("Cancel");
    expect(cancel.getAttribute("type")).toBe("button");
    expect(layersOf(cancel)).toEqual(["false", "false", "true"]);
    expect(innerOf(cancel)).toEqual(["false", "true"]);

    act(() => {
      useTurnStore.setState({
        byId: new Map([
          ["session-1", { ...EMPTY_SLICE, runState: "cancelling" }],
        ]),
      });
    });
    expect(innerOf(screen.getByTitle("Cancel"))).toEqual(["true", "false"]);

    fireEvent.change(screen.getByPlaceholderText("Message the agent..."), {
      target: { value: "steer" },
    });
    const queue = screen.getByTitle("Queue for the next step");
    expect(layersOf(queue)).toEqual(["false", "true", "false"]);
    // One node throughout: the swap only moves the glyphs, so focus and the
    // keyboard press feedback (sendBtnRef → .send-press) survive it.
    expect(queue).toBe(send);
  });

  it("appends a recalled queued batch to the draft instead of replacing it", () => {
    render(<AgentChatInput sessionId="session-1" />);
    const textarea = screen.getByPlaceholderText(
      "Message the agent...",
    ) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "my draft" } });

    act(() => {
      appendComposerText("session-1", "queued one\n\nqueued two");
    });
    expect(textarea.value).toBe("my draft\n\nqueued one\n\nqueued two");

    // Another session's recall never lands in this composer.
    act(() => {
      appendComposerText("session-2", "elsewhere");
    });
    expect(textarea.value).toBe("my draft\n\nqueued one\n\nqueued two");
  });

  it("fills an empty draft with the recalled text verbatim", () => {
    render(<AgentChatInput sessionId="session-1" />);
    const textarea = screen.getByPlaceholderText(
      "Message the agent...",
    ) as HTMLTextAreaElement;

    act(() => {
      appendComposerText("session-1", "queued");
    });
    expect(textarea.value).toBe("queued");
  });

  it("keeps the draft and does not toast when start rejects", () => {
    useConnectionStore.setState({
      state: "connected",
      sendRpc: undefined,
    } as never);
    render(<AgentChatInput sessionId="session-1" />);

    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(useToastStore.getState().toasts).toEqual([]);
    expect((textarea as HTMLTextAreaElement).value).toBe("hello");
    expect(
      useTurnStore.getState().byId.get("session-1")?.runState ?? "idle",
    ).toBe("idle");
  });
});

describe("AgentChatInput — subagent variant", () => {
  it("keeps model / thinking tier / context mode + the usage ring", () => {
    render(<AgentChatInput sessionId="session-1" variant="subagent" />);

    expect(screen.getByTestId("subagent-controls")).toBeTruthy();
    expect(screen.getByTitle("Model: Model 1")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Med" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Default" })).toBeTruthy();
    expect(screen.getByTitle("Context usage")).toBeTruthy();
  });

  it("mounts no composer: no textarea, no send/cancel, no notification bell", () => {
    render(<AgentChatInput sessionId="session-1" variant="subagent" />);

    expect(document.querySelector("textarea")).toBeNull();
    expect(screen.queryByPlaceholderText("Message the agent...")).toBeNull();
    expect(screen.queryByTitle("Send")).toBeNull();
    expect(screen.queryByTitle("Cancel")).toBeNull();
  });

  it("drops the agent picker: a child's identity is fixed by its profile", () => {
    useSessionStore.setState({
      primaryAgents: [{ id: "default", description: "" }],
    } as never);

    const primary = render(<AgentChatInput sessionId="session-1" />);
    expect(screen.getByTitle("default")).toBeTruthy();
    primary.unmount();

    render(<AgentChatInput sessionId="session-1" variant="subagent" />);
    expect(screen.queryByTitle("default")).toBeNull();
  });

  it("keeps the ring but drops its Compaction action (read-only)", () => {
    const primary = render(<AgentChatInput sessionId="session-1" />);
    fireEvent.click(screen.getByTitle("Context usage"));
    expect(screen.getByLabelText("Compact context")).toBeTruthy();
    primary.unmount();

    render(<AgentChatInput sessionId="session-1" variant="subagent" />);
    fireEvent.click(screen.getByTitle("Context usage"));
    expect(screen.getByText("No context usage yet")).toBeTruthy();
    expect(screen.queryByLabelText("Compact context")).toBeNull();
  });
});

describe("AgentChatInput model gate", () => {
  /** A session the server has not repaired yet: no stored model. */
  function dropSessionModel() {
    const byId = new Map(useSessionStore.getState().byId);
    const current = byId.get("session-1");
    byId.set("session-1", {
      ...(current as object),
      modelId: null,
      apiModelId: null,
      label: "",
    } as never);
    useSessionStore.setState({ byId } as never);
  }

  it("sends when the session has no model yet but the catalog has one", async () => {
    dropSessionModel();
    render(<AgentChatInput sessionId="session-1" />);
    fireEvent.change(document.querySelector("textarea")!, {
      target: { value: "hello" },
    });

    const send = screen.getByTitle("Send") as HTMLButtonElement;
    expect(send.disabled).toBe(false);
    fireEvent.click(send);

    await waitFor(() => {
      expect(useTurnStore.getState().byId.get("session-1")?.runState).toBe(
        "running",
      );
    });
  });

  it("stays blocked when no model exists at all", () => {
    dropSessionModel();
    useSessionStore.setState({ availableModels: [] } as never);
    render(<AgentChatInput sessionId="session-1" />);
    const ta = document.querySelector("textarea")!;
    fireEvent.change(ta, { target: { value: "hello" } });

    expect((screen.getByTitle("Send") as HTMLButtonElement).disabled).toBe(true);
    expect(
      screen.getByPlaceholderText("Add a model in Settings first..."),
    ).toBeTruthy();
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(useTurnStore.getState().byId.get("session-1")?.runState ?? "idle").toBe(
      "idle",
    );
    expect(useConnectionStore.getState().sendRpc).not.toHaveBeenCalled();
  });
});

describe("AgentChatInput images", () => {
  beforeEach(() => {
    vi.mocked(uploadMedia).mockResolvedValue({
      ref: MEDIA_REF,
      mime: "image/jpeg",
      width: 4,
      height: 4,
    });
    URL.createObjectURL = vi.fn(() => "blob:preview");
    URL.revokeObjectURL = vi.fn();
  });

  async function pasteShot(): Promise<HTMLTextAreaElement> {
    const file = new File([new Uint8Array([1, 2, 3])], "shot.png", {
      type: "image/png",
    });
    const textarea = screen.getByPlaceholderText(
      "Message the agent...",
    ) as HTMLTextAreaElement;
    fireEvent.paste(textarea, {
      clipboardData: {
        items: [
          {
            kind: "file",
            type: "image/png",
            getAsFile: () => file,
          },
        ],
        files: [file],
      },
    });
    await waitFor(() => expect(uploadMedia).toHaveBeenCalled());
    await screen.findByTestId("composer-images");
    return textarea;
  }

  function allowImages() {
    useSessionStore.setState({
      availableModels: [
        {
          id: "openai/model-1",
          api_model_id: "model-1",
          provider_id: "openai",
          label: "Model 1",
          context_window: 1000,
          modalities: ["text", "image"],
        },
      ],
    } as never);
  }

  it("blocks send and masks the image when the model has no image modality", async () => {
    render(<AgentChatInput sessionId="session-1" />);
    const textarea = await pasteShot();
    expect(screen.getByText("Unsupported")).toBeTruthy();
    const send = screen.getByTitle(
      "Switch to a model that supports images",
    ) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });
    expect(useConnectionStore.getState().sendRpc).not.toHaveBeenCalled();
    expect(useToastStore.getState().toasts.map((toast) => toast.message)).toContain(
      "Switch to a model that supports images",
    );
  });

  it("sends an image on its own when the model accepts images", async () => {
    allowImages();
    render(<AgentChatInput sessionId="session-1" />);
    const textarea = await pasteShot();
    expect(screen.queryByText("Unsupported")).toBeNull();
    const send = screen.getByTitle("Send") as HTMLButtonElement;
    expect(send.disabled).toBe(false);
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });
    await waitFor(() => {
      expect(useConnectionStore.getState().sendRpc).toHaveBeenCalledWith(
        "agent/run",
        expect.objectContaining({
          input: "",
          session_id: "session-1",
          images: [{ ref: MEDIA_REF }],
        }),
      );
    });
  });

  it("puts recalled images back on the draft", () => {
    render(<AgentChatInput sessionId="session-1" />);
    act(() => {
      appendComposerText("session-1", "look", [MEDIA_REF]);
    });
    expect(screen.getByTestId("composer-images")).toBeTruthy();
    expect(
      (screen.getByPlaceholderText("Message the agent...") as HTMLTextAreaElement)
        .value,
    ).toBe("look");
    expect(screen.getByText("Unsupported")).toBeTruthy();
  });
});
