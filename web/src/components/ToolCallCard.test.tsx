import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  deriveToolStatus,
  isToolCallLive,
  processGroupAutoOpen,
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

  it("treats edit partial-success as warning and Hint-only as ok", () => {
    const partial: FunctionCallOutputItem = {
      type: "function_call_output",
      call_id: "c1",
      output:
        "Edited src/a.rs (1 applied / 0 warning / 1 failed). File updated.\n\nWarning: some edits were not applied",
    };
    expect(deriveToolStatus(partial, false)).toBe("warning");
    const hintOnly: FunctionCallOutputItem = {
      type: "function_call_output",
      call_id: "c1",
      output:
        "Edited src/a.rs (1 applied / 0 warning / 0 failed). File updated.\n\nHint: LSP note — rust-analyzer\nError: missing semicolon",
    };
    expect(deriveToolStatus(hintOnly, false)).toBe("ok");
  });
});

describe("isToolCallLive", () => {
  it("stays live after the call seq is sealed until output arrives", () => {
    expect(isToolCallLive({ callStatus: "completed", hasOutput: false })).toBe(true);
  });

  it("stays live while the call seq itself is in_progress", () => {
    expect(isToolCallLive({ callStatus: "in_progress", hasOutput: false })).toBe(true);
  });

  it("stays live while the output seq is still in_progress", () => {
    expect(
      isToolCallLive({ callStatus: "completed", hasOutput: true, outputInProgress: true }),
    ).toBe(true);
  });

  it("is not live once the output seq exists and is sealed", () => {
    expect(isToolCallLive({ callStatus: "completed", hasOutput: true })).toBe(false);
  });

  it("is not live when the call failed or is incomplete (no output expected)", () => {
    expect(isToolCallLive({ callStatus: "failed", hasOutput: false })).toBe(false);
    expect(isToolCallLive({ callStatus: "incomplete", hasOutput: false })).toBe(false);
  });
});

describe("processGroupAutoOpen", () => {
  it("stays open between completed tool-loop steps", () => {
    expect(
      processGroupAutoOpen({ followedByMessage: false, hasTerminalStop: false }),
    ).toBe(true);
  });

  it("collapses once the following assistant message closes the segment", () => {
    expect(
      processGroupAutoOpen({ followedByMessage: true, hasTerminalStop: false }),
    ).toBe(false);
  });

  it("collapses on a terminal failure without a following message", () => {
    expect(
      processGroupAutoOpen({ followedByMessage: false, hasTerminalStop: true }),
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

  it("shows edit header as file + line-level +N/−M diff", () => {
    const editCall: FunctionCallItem = {
      type: "function_call",
      id: "fc_edit",
      call_id: "call_edit",
      name: "edit",
      arguments: JSON.stringify({
        file_path: "src/a.ts",
        old_string: "foo\nbar\nbaz",
        new_string: "foo\nqux\nbaz\nquux",
      }),
      status: "completed",
    };
    renderCard({ call: editCall, output });
    const header = screen.getByRole("button", { name: /edit/i });
    expect(header.textContent).toContain("src/a.ts");
    expect(header.textContent).toContain("+2");
    expect(header.textContent).toContain("−1");
  });

  it("sums +N/−M across edits[] request previews", () => {
    const editCall: FunctionCallItem = {
      type: "function_call",
      id: "fc_edit_batch",
      call_id: "call_edit_batch",
      name: "edit",
      arguments: JSON.stringify({
        file_path: "src/a.ts",
        edits: [
          { old_string: "foo\nbar\nbaz", new_string: "foo\nqux\nbaz\nquux" },
          { old_string: "a", new_string: "b\nc" },
        ],
      }),
      status: "completed",
    };
    renderCard({ call: editCall, output });
    const header = screen.getByRole("button", { name: /edit/i });
    expect(header.textContent).toContain("src/a.ts");
    expect(header.textContent).toContain("+");
  });

  it("mounts a live tool card collapsed by default", () => {
    renderCard({ streaming: true });
    const header = screen.getByRole("button", { name: /write/i });
    expect(header.getAttribute("aria-expanded")).toBe("false");
  });

  it("shows Kill on a live bash card and calls bash/kill", () => {
    const sendRpc = vi.fn(async () => ({ ok: true }));
    useConnectionStore.setState({ sendRpc } as never);
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
    expect(screen.getByRole("button", { name: /^Kill$/ })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /^Kill$/ }));
    expect(sendRpc).toHaveBeenCalledWith("bash/kill", { bash_id: "bg_a" });
  });
});