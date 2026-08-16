import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  deriveToolStatus,
  isToolCallLive,
  processGroupStreaming,
} from "./toolCallStatus";
import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { useBashStore } from "../stores/bashStore";
import { useConnectionStore } from "../stores/connectionStore";
import { ToolCallCard } from "./ToolCallCard";
import { clearFoldCardOpen } from "./foldCardState";

vi.mock("../stores/messageStore", () => ({
  useMessageStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({ bySession: new Map() }),
}));

vi.mock("../stores/turnStore", () => ({
  useTurnStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({ byId: new Map() }),
}));

// Force the fallback content renderer so tool views (bash/write/subagent) with
// heavier dependencies never mount in jsdom.
vi.mock("./toolviews/registry", () => ({
  getToolView: () => undefined,
  viewOwnsOutput: () => false,
}));

afterEach(() => {
  cleanup();
  // foldCardState is module-global; the same foldCardId is reused across
  // cases, so drop it between tests (production ids are unique per bubble).
  clearFoldCardOpen("session-1");
  useBashStore.getState().reset();
});

describe("deriveToolStatus", () => {
  it("is running while streaming and no output yet", () => {
    expect(deriveToolStatus(undefined, true)).toBe("running");
  });

  it("is unknown when idle with no output (turn finished / orphan call)", () => {
    expect(deriveToolStatus(undefined, false)).toBe("unknown");
  });

  it("is ok when output is present", () => {
    const output: FunctionCallOutputItem = {
      type: "function_call_output",
      call_id: "c1",
      output: "done",
    };
    expect(deriveToolStatus(output, false)).toBe("ok");
  });

  it("is failed when output starts with Error:", () => {
    const output: FunctionCallOutputItem = {
      type: "function_call_output",
      call_id: "c1",
      output: "Error: boom",
    };
    expect(deriveToolStatus(output, true)).toBe("failed");
  });

  it("is failed when the call Item status is 'failed' (stream invalidation), even with no output (FE-06)", () => {
    expect(deriveToolStatus(undefined, false, "failed")).toBe("failed");
    expect(deriveToolStatus(undefined, true, "failed")).toBe("failed");
  });

  it("is warning when output starts with Warning:", () => {
    const output: FunctionCallOutputItem = {
      type: "function_call_output",
      call_id: "c1",
      output: "Warning: exit_code 1. Hint: inspect output",
    };
    expect(deriveToolStatus(output, false)).toBe("warning");
  });

  it("is warning when Warning: is appended after a success body", () => {
    const output: FunctionCallOutputItem = {
      type: "function_call_output",
      call_id: "c1",
      output: "Created: a.rs\n\nWarning: language engine is still warming. Hint: retry",
    };
    expect(deriveToolStatus(output, false)).toBe("warning");
  });
});

describe("isToolCallLive", () => {
  it("stays live after early seal while turn is active and output is missing", () => {
    expect(isToolCallLive(false, false, true)).toBe(true);
  });

  it("is not live after turn ends with sealed call and no output", () => {
    expect(isToolCallLive(false, false, false)).toBe(false);
  });

  it("stops being live once output exists even if turn is still active", () => {
    expect(isToolCallLive(true, false, true)).toBe(false);
  });

  it("stays live while the call row itself is still streaming", () => {
    expect(isToolCallLive(false, true, false)).toBe(true);
  });
});

describe("processGroupStreaming", () => {
  it("stays open while turn runs and no text has followed yet", () => {
    expect(
      processGroupStreaming({ hasTextAfter: false, turnActive: true }),
    ).toBe(true);
  });

  it("collapses once an assistant text block follows the process group", () => {
    expect(
      processGroupStreaming({ hasTextAfter: true, turnActive: true }),
    ).toBe(false);
  });

  it("collapses when the turn ends even without text after", () => {
    expect(
      processGroupStreaming({ hasTextAfter: false, turnActive: false }),
    ).toBe(false);
  });

  it("stays collapsed when turn ended and text already followed", () => {
    expect(
      processGroupStreaming({ hasTextAfter: true, turnActive: false }),
    ).toBe(false);
  });
});

describe("ToolCallCard open state", () => {
  const sealedWriteCall: FunctionCallItem = {
    type: "function_call",
    id: "fc_1",
    call_id: "call_1",
    name: "write",
    arguments: JSON.stringify({ file_path: "a.txt", content: "hello" }),
    status: "completed",
  };
  const output: FunctionCallOutputItem = {
    type: "function_call_output",
    call_id: "call_1",
    output: "Created a.txt",
  };

  function renderCard(props: {
    call?: FunctionCallItem;
    output?: FunctionCallOutputItem;
    streaming?: boolean;
  } = {}) {
    return render(
      <ToolCallCard
        call={props.call ?? sealedWriteCall}
        output={props.output}
        streaming={props.streaming}
        projectRoot={null}
        onOpenFile={() => {}}
        sessionId="session-1"
        foldCardId="session-1:bubble:tool:call_1"
      />,
    );
  }

  it("mounts a sealed edit/bash/write card collapsed (no forced defaultOpen)", () => {
    renderCard({ output });
    const header = screen.getByRole("button", { name: /write/i });
    expect(header.getAttribute("aria-expanded")).toBe("false");
  });

  it("mounts a live tool card expanded", () => {
    renderCard({ streaming: true });
    const header = screen.getByRole("button", { name: /write/i });
    expect(header.getAttribute("aria-expanded")).toBe("true");
  });

  it("shows Kill on a live bash card and calls bash/kill", () => {
    const sendRpc = vi.fn(async () => ({ ok: true }));
    useConnectionStore.setState({ sendRpc });
    useBashStore.getState().applySnapshot("session-1", {
      jobs: [
        {
          id: "bg_a",
          call_id: "call_1",
          command_preview: "sleep 8",
          output_file: ".litecode/bash/bg_a.output",
          started_at_ms: Date.now() - 12_000,
        },
      ],
      waits: [],
    });
    render(
      <ToolCallCard
        call={{
          type: "function_call",
          id: "fc_bash",
          call_id: "call_1",
          name: "bash",
          arguments: JSON.stringify({ command: "sleep 8" }),
          status: "completed",
        }}
        output={{
          type: "function_call_output",
          call_id: "call_1",
          output: "status: running\nbash_id: bg_a\n",
        }}
        projectRoot={null}
        onOpenFile={() => {}}
        sessionId="session-1"
        foldCardId="session-1:bubble:tool:call_1"
      />,
    );
    expect(screen.getByText("running 1")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /^Kill$/ }));
    expect(sendRpc).toHaveBeenCalledWith("bash/kill", { bash_id: "bg_a" });
  });
});