import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BashToolView } from "./BashToolView";
import { useBashStore } from "../../stores/bashStore";
import { useConnectionStore } from "../../stores/connectionStore";

afterEach(() => {
  cleanup();
  useBashStore.getState().reset();
});

describe("BashToolView live overlay", () => {
  it("overlays tee tail while the matched job is alive and ignores sealed running text", async () => {
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "bash/tail") {
        return {
          text: "live-out",
          truncated_on_disk: false,
          alive: true,
          exit_code: null,
        };
      }
      throw new Error(`unexpected ${method}`);
    });
    useConnectionStore.setState({ sendRpc });
    useBashStore.getState().applySnapshot("s1", {
      jobs: [
        {
          id: "bg_a",
          call_id: "call_1",
          command_preview: "sleep 8",
          output_file: ".litecode/bash/bg_a.output",
          started_at_ms: Date.now(),
        },
      ],
      waits: [],
    });

    render(
      <BashToolView
        name="bash"
        status="ok"
        input={{ command: "sleep 8" }}
        output={{
          type: "function_call_output",
          call_id: "call_1",
          output: "status: running\nbash_id: bg_a\noutput_file: .litecode/bash/bg_a.output\n",
        }}
        call_id="call_1"
        sessionId="s1"
      />,
    );

    expect(await screen.findByText("live-out")).toBeTruthy();
    expect(screen.queryByText(/status: running/)).toBeNull();
    await waitFor(() => {
      expect(sendRpc).toHaveBeenCalledWith("bash/tail", { bash_id: "bg_a" });
    });
  });
});
