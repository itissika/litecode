import { describe, expect, it } from "vitest";

import type { FunctionCallOutputItem } from "../api/types";
import {
  deriveToolStatus,
  isToolCallLive,
  processGroupAutoOpen,
} from "./toolCallStatus";

const result = (output: string): FunctionCallOutputItem => ({
  type: "function_call_output",
  call_id: "call_1",
  output,
});

describe("deriveToolStatus", () => {
  it("marks a NON-zero leading exit_code as a warning (the command failed, not the call)", () => {
    expect(
      deriveToolStatus(result("exit_code: 1\nboom\n"), false, "completed"),
    ).toBe("warning");
    expect(
      deriveToolStatus(result("exit_code: 130\n"), false, "completed"),
    ).toBe("warning");
  });

  it("keeps a zero exit_code neutral", () => {
    expect(
      deriveToolStatus(result("exit_code: 0\nall good\n"), false, "completed"),
    ).toBe("ok");
  });

  it("does not read an exit_code that is not the leading line", () => {
    // e.g. `read` of a file whose content mentions exit_code, or bash output
    // whose first line is something else.
    expect(
      deriveToolStatus(result("note\nexit_code: 1\n"), false, "completed"),
    ).toBe("ok");
  });

  it("keeps a sealed background-bash running document neutral", () => {
    expect(
      deriveToolStatus(
        result(
          "status: running\nbash_id: bg_a\noutput_file: .litecode/bash/bg_a.output\n",
        ),
        false,
        "completed",
      ),
    ).toBe("ok");
  });

  it("keeps the existing Error:/Warning: prefixes and lifecycle statuses", () => {
    expect(deriveToolStatus(result("Error: nope"), false, "completed")).toBe(
      "failed",
    );
    expect(
      deriveToolStatus(result("Warning: careful"), false, "completed"),
    ).toBe("warning");
    expect(deriveToolStatus(undefined, true, "in_progress")).toBe("running");
    expect(deriveToolStatus(undefined, false, "failed")).toBe("failed");
    expect(deriveToolStatus(undefined, false, "completed")).toBe("unknown");
  });
});

describe("isToolCallLive / processGroupAutoOpen", () => {
  it("stays live until a matching output exists", () => {
    expect(isToolCallLive({ callStatus: "completed", hasOutput: false })).toBe(
      true,
    );
    expect(isToolCallLive({ callStatus: "completed", hasOutput: true })).toBe(
      false,
    );
    expect(
      isToolCallLive({
        callStatus: "completed",
        hasOutput: true,
        outputInProgress: true,
      }),
    ).toBe(true);
    expect(isToolCallLive({ callStatus: "failed", hasOutput: false })).toBe(
      false,
    );
  });

  it("auto-opens a process group unless it is closed by a message or a terminal stop", () => {
    expect(
      processGroupAutoOpen({
        followedByMessage: false,
        hasTerminalStop: false,
      }),
    ).toBe(true);
    expect(
      processGroupAutoOpen({ followedByMessage: true, hasTerminalStop: false }),
    ).toBe(false);
    expect(
      processGroupAutoOpen({ followedByMessage: false, hasTerminalStop: true }),
    ).toBe(false);
  });
});
