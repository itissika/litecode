import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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
    useConnectionStore.setState({ sendRpc } as never);
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
          output:
            "status: running\nbash_id: bg_a\noutput_file: .litecode/bash/bg_a.output\n",
        }}
        call_id="call_1"
        sessionId="s1"
      />,
    );

    expect(await screen.findByText("live-out")).toBeTruthy();
    expect(screen.queryByText(/status: running/)).toBeNull();
    expect(screen.getByTestId("bash-console")).toBeTruthy();
    expect(screen.getByText("sleep 8")).toBeTruthy();
    const outputPre = screen.getByText("live-out").closest("pre");
    expect(outputPre?.className).toContain("overflow-hidden");
    await waitFor(() => {
      expect(sendRpc).toHaveBeenCalledWith("bash/tail", { bash_id: "bg_a" });
    });
  });

  it("expands a multiline command in place", () => {
    render(
      <BashToolView
        name="bash"
        status="ok"
        input={{ command: "npm run build\nnpm run test" }}
        output={{
          type: "function_call_output",
          call_id: "call_1",
          output: "ok\n",
        }}
        call_id="call_1"
        sessionId="s1"
      />,
    );

    const expand = screen.getByRole("button", { name: "Expand command" });
    fireEvent.click(expand);
    expect(
      screen.getByRole("button", { name: "Collapse command" }),
    ).toBeTruthy();
    expect(screen.getByText(/npm run build[\s\S]*npm run test/)).toBeTruthy();
  });

  it("settles the overlay on the real exit and reports the polled exit code", async () => {
    // Sealed background result: `status: running` is a one-way seal, so the only
    // authority on liveness is the poll — and it must keep polling after the job
    // leaves the snapshot instead of freezing the last sample on screen.
    let calls = 0;
    const sendRpc = vi.fn(async (method: string) => {
      if (method !== "bash/tail") throw new Error(`unexpected ${method}`);
      calls += 1;
      return calls === 1
        ? {
            text: "live-out",
            truncated_on_disk: false,
            alive: true,
            exit_code: null,
          }
        : {
            text: "final-out",
            truncated_on_disk: false,
            alive: false,
            exit_code: 3,
          };
    });
    useConnectionStore.setState({ sendRpc } as never);
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
          output:
            "status: running\nbash_id: bg_a\noutput_file: .litecode/bash/bg_a.output\n",
        }}
        call_id="call_1"
        sessionId="s1"
      />,
    );

    expect(await screen.findByText("live-out")).toBeTruthy();

    // Job gone from the snapshot: liveness now comes from the poll alone.
    useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });

    await waitFor(() => {
      expect(screen.getByTestId("bash-footer").textContent).toContain(
        "exited  exit_code: 3",
      );
    });
    expect(screen.queryByText("live-out")).toBeNull();
    expect(screen.queryByText("final-out")).toBeNull();
    // The sealed document still points at the full log.
    expect(screen.getByTestId("bash-output-file").textContent).toBe(
      ".litecode/bash/bg_a.output",
    );
  });
});

describe("BashToolView parses the real backend result document", () => {
  const renderOutput = (output: string) =>
    render(
      <BashToolView
        name="bash"
        status="ok"
        input={{ command: "npm test" }}
        output={{ type: "function_call_output", call_id: "call_1", output }}
        call_id="call_1"
        sessionId="s1"
      />,
    );

  it("reads exit_code from the FIRST line and the frozen head/tail window", () => {
    renderOutput(
      "exit_code: 1\n" +
        "bytes: 9000\n" +
        "output_file: .litecode/bash/bg_1.output\n" +
        "truncated_on_disk: true\n" +
        "head 2048B + tail 4096B of 9000 bytes. output_file has the full log.\n\n" +
        "--- head ---\n" +
        "test 1 FAILED\n" +
        "--- tail ---\n" +
        "2 tests failed\n",
    );

    expect(screen.getByText(/test 1 FAILED/)).toBeTruthy();
    expect(screen.getByTestId("bash-tail").textContent).toBe("2 tests failed");
    expect(screen.getByTestId("bash-footer").textContent).toBe("exit_code: 1");
    expect(screen.getByTestId("bash-window-note").textContent).toContain(
      "9000 bytes",
    );
    expect(screen.getByTestId("bash-window-note").textContent).toContain(
      ".litecode/bash/bg_1.output",
    );
    expect(screen.getByTestId("bash-window-note").textContent).toContain(
      "truncated on disk",
    );
  });

  it("treats a small capture as plain output (no head/tail markers, no exit_code)", () => {
    renderOutput("just output\nmore output\n");

    expect(screen.getByText(/just output[\s\S]*more output/)).toBeTruthy();
    expect(screen.queryByTestId("bash-tail")).toBeNull();
    expect(screen.queryByTestId("bash-footer")).toBeNull();
  });

  it("renders a cancelled run as a status footer", () => {
    renderOutput(
      "status: cancelled\n\n--- head ---\ninterrupted\n--- tail ---\n",
    );

    expect(screen.getByText("interrupted")).toBeTruthy();
    expect(screen.getByTestId("bash-footer").textContent).toBe(
      "status: cancelled",
    );
  });

  it("keeps exit_code: 0 neutral in the footer", () => {
    renderOutput("exit_code: 0\nall good\n");

    expect(screen.getByTestId("bash-footer").textContent).toBe("exit_code: 0");
    expect(screen.getByTestId("bash-footer").className).not.toContain("amber");
  });
});
