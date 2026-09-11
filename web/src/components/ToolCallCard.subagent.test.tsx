import { act, cleanup, render, type RenderResult } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { ToolCallCard } from "./ToolCallCard";
import { clearFoldCardOpen } from "./foldCardState";
import { useBashStore } from "../stores/bashStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";

/**
 * subagent_launch card behavior: label contents, child-session subscription
 * ownership (lives on the card, not the body view), and the header presence
 * icon state machine (static → pop on running entry → breathing → settle
 * pop on ok / generic fail on error).
 */

const PARENT = "parent-s";
const CHILD = "child-s";
const CALL_ID = "call_a";
const CARD_ID = "parent-s:bubble:tool:call_a";

function subagentCall(): FunctionCallItem {
  return {
    type: "function_call",
    id: "fc_sa",
    call_id: CALL_ID,
    name: "subagent_launch",
    arguments: JSON.stringify({ agent: "explore", prompt: "do the thing" }),
    status: "completed",
  };
}

function seedBinding(): void {
  useMessageStore.getState().onSubagentBound(PARENT, {
    session_id: PARENT,
    call_id: CALL_ID,
    child_session_id: CHILD,
  });
}

function seedChildRunState(runState: "idle" | "running" | "cancelling"): void {
  act(() => {
    useTurnStore.setState((s) => ({
      byId: new Map(s.byId).set(CHILD, { ...EMPTY_SLICE, runState }),
    }));
  });
}

function seedChildText(text: string, seq = 100): void {
  act(() => {
    useMessageStore.getState().onBufferItem(CHILD, {
      session_id: CHILD,
      seq,
      kind: "item/assistant",
      body: {
        type: "message",
        role: "assistant",
        id: `msg_${seq}`,
        status: "in_progress",
        content: [{ type: "output_text", text, annotations: [] }],
      },
    });
  });
}

function renderSubagentCard(opts: {
  output?: FunctionCallOutputItem;
  streaming?: boolean;
} = {}): RenderResult {
  return render(
    <ToolCallCard
      call={subagentCall()}
      output={opts.output}
      streaming={opts.streaming ?? true}
      projectRoot={null}
      onOpenFile={() => {}}
      sessionId={PARENT}
      foldCardId={CARD_ID}
    />,
  );
}

const presence = (): HTMLElement | null =>
  document.querySelector(".sa-presence");

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  clearFoldCardOpen(CARD_ID);
  useConnectionStore.setState({ state: "disconnected" });
  useMessageStore.getState().reset(PARENT);
  useMessageStore.getState().reset(CHILD);
  useTurnStore.getState().resetTurn(PARENT);
  useTurnStore.getState().resetTurn(CHILD);
  useBashStore.getState().reset(PARENT);
  useBashStore.getState().reset(CHILD);
});

describe("subagent_launch card header", () => {
  it("labels the card with the agent name (no tool name) and stays static before running", () => {
    seedBinding();
    renderSubagentCard();

    const header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("explore");
    expect(header.textContent).not.toContain("subagent_launch");
    expect(presence()?.getAttribute("title")).toBe("Launching");
    expect(presence()?.classList.contains("sa-running")).toBe(false);
    expect(presence()?.classList.contains("tool-icon--pop")).toBe(false);
  });

  it("falls back to a muted placeholder when the agent is unknown", () => {
    render(
      <ToolCallCard
        call={{
          type: "function_call",
          id: "fc_sa",
          call_id: CALL_ID,
          name: "subagent_launch",
          arguments: JSON.stringify({ prompt: "do the thing" }),
          status: "completed",
        }}
        streaming={true}
        projectRoot={null}
        onOpenFile={() => {}}
        sessionId={PARENT}
        foldCardId={CARD_ID}
      />,
    );
    const header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("subagent");
  });

  it("plays no settle pop when a finished card is mounted (remount-safe)", () => {
    seedBinding();
    seedChildRunState("idle");
    renderSubagentCard({
      output: { type: "function_call_output", call_id: CALL_ID, output: "done" },
      streaming: false,
    });
    const el = presence();
    expect(el?.getAttribute("title")).toBe("Completed");
    expect(el?.classList.contains("tool-icon--pop")).toBe(false);
    expect(el?.classList.contains("sa-running")).toBe(false);
  });
});

describe("subagent_launch card subscription ownership", () => {
  it("subscribes the bound child session for the card's whole lifetime", () => {
    useConnectionStore.setState({ state: "connected" });
    seedBinding();
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    renderSubagentCard();

    expect(subscribe).toHaveBeenCalledWith(CHILD);
  });

  it("does not subscribe before a binding exists", () => {
    useConnectionStore.setState({ state: "connected" });
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    renderSubagentCard();

    expect(subscribe).not.toHaveBeenCalled();
  });

  it("unsubscribes and clears the child slices on unmount", () => {
    useConnectionStore.setState({ state: "connected" });
    seedBinding();
    seedChildRunState("running");
    vi.spyOn(useConnectionStore.getState(), "ensureSubscribe").mockResolvedValue(
      undefined,
    );
    useBashStore.getState().applySnapshot(CHILD, {
      jobs: [
        {
          id: "bg_x",
          call_id: "call_b",
          command_preview: "sleep 300",
          output_file: ".litecode/bash/bg_x.output",
          started_at_ms: Date.now() - 5000,
        },
      ],
      waits: [],
    });

    const { unmount } = renderSubagentCard();
    const unsub = vi.spyOn(useConnectionStore.getState(), "unsubscribeSession");
    unmount();

    expect(unsub).toHaveBeenCalledWith(CHILD);
    expect(useTurnStore.getState().byId.get(CHILD)?.runState).toBe("idle");
    expect(useBashStore.getState().bySession.get(CHILD)).toBeUndefined();
  });
});

describe("subagent_launch card presence icon state machine", () => {
  it("pops once on idle → running, then settles into breathing", () => {
    vi.useFakeTimers();
    seedBinding();
    seedChildRunState("idle");
    renderSubagentCard();

    seedChildRunState("running");

    const el = presence();
    expect(el?.classList.contains("tool-icon--pop")).toBe(true);
    expect(el?.classList.contains("sa-running")).toBe(false);
    expect(el?.getAttribute("title")).toBe("Running");

    act(() => {
      vi.advanceTimersByTime(800);
    });

    const after = presence();
    expect(after?.classList.contains("tool-icon--pop")).toBe(false);
    expect(after?.classList.contains("sa-running")).toBe(true);
    expect(after?.getAttribute("title")).toBe("Running");
  });

  it("mounting mid-run goes straight to breathing (no replay pop)", () => {
    seedBinding();
    seedChildRunState("running");
    renderSubagentCard();

    const el = presence();
    expect(el?.classList.contains("sa-running")).toBe(true);
    expect(el?.classList.contains("tool-icon--pop")).toBe(false);
  });

  it("sealing with ok pops once (agent settle), then turns static", () => {
    vi.useFakeTimers();
    seedBinding();
    seedChildRunState("running");
    const view = renderSubagentCard();
    expect(presence()?.classList.contains("sa-running")).toBe(true);

    view.rerender(
      <ToolCallCard
        call={subagentCall()}
        output={{
          type: "function_call_output",
          call_id: CALL_ID,
          output: "done",
        }}
        streaming={false}
        projectRoot={null}
        onOpenFile={() => {}}
        sessionId={PARENT}
        foldCardId={CARD_ID}
      />,
    );

    let el = presence();
    expect(el?.getAttribute("title")).toBe("Completed");
    expect(el?.classList.contains("tool-icon--pop")).toBe(true);
    expect(el?.classList.contains("sa-running")).toBe(false);
    expect(document.querySelector(".tool-icon-glow")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(800);
    });

    el = presence();
    expect(el?.classList.contains("tool-icon--pop")).toBe(false);
    expect(el?.classList.contains("sa-failed")).toBe(false);
    expect(document.querySelector(".tool-icon-glow")).toBeNull();
  });

  it("sealing with an Error: output plays the generic red failure", () => {
    vi.useFakeTimers();
    seedBinding();
    seedChildRunState("running");
    const view = renderSubagentCard();

    view.rerender(
      <ToolCallCard
        call={subagentCall()}
        output={{
          type: "function_call_output",
          call_id: CALL_ID,
          output: "Error: subagent cancelled",
        }}
        streaming={false}
        projectRoot={null}
        onOpenFile={() => {}}
        sessionId={PARENT}
        foldCardId={CARD_ID}
      />,
    );

    let el = presence();
    expect(el?.classList.contains("tool-icon--fail-anim")).toBe(true);
    expect(el?.classList.contains("sa-failed")).toBe(true);
    expect(document.querySelector(".tool-icon-shockwave")).toBeTruthy();
    expect(el?.getAttribute("title")).toBe("Failed");

    act(() => {
      vi.advanceTimersByTime(1000);
    });

    el = presence();
    expect(el?.classList.contains("tool-icon--fail-anim")).toBe(false);
    expect(el?.classList.contains("sa-failed")).toBe(true);
  });
});

describe("subagent_launch card header progress summary", () => {
  it("shows the latest child assistant text while running", () => {
    seedBinding();
    seedChildRunState("running");
    seedChildText("reading the spec first");
    renderSubagentCard();

    const header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("explore");
    expect(header.textContent).toContain("reading the spec first");
  });

  it("rolls the header text up as newer child text arrives", () => {
    seedBinding();
    seedChildRunState("running");
    seedChildText("step one", 100);
    renderSubagentCard();

    let header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("step one");

    seedChildText("step two", 101);

    header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).not.toContain("step one");
    expect(header.textContent).toContain("step two");
  });

  it("keeps just the agent name before any child text exists", () => {
    seedBinding();
    seedChildRunState("running");
    renderSubagentCard();

    const header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("explore");
    expect(header.textContent).not.toContain("completed");
    expect(header.textContent).not.toContain("failed");
  });

  it("swaps to a completed status once the card is sealed with ok", () => {
    seedBinding();
    seedChildRunState("idle");
    seedChildText("final answer");
    renderSubagentCard({
      output: { type: "function_call_output", call_id: CALL_ID, output: "done" },
      streaming: false,
    });

    const header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("completed");
    expect(header.textContent).not.toContain("final answer");
  });

  it("swaps to a failed status when the sealed output is an error", () => {
    seedBinding();
    seedChildRunState("idle");
    renderSubagentCard({
      output: {
        type: "function_call_output",
        call_id: CALL_ID,
        output: "Error: subagent crashed",
      },
      streaming: false,
    });

    const header = document.querySelector(".foldcard-header") as HTMLElement;
    expect(header.textContent).toContain("failed");
  });
});
