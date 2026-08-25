import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { collectEditBlocks, EditToolView } from "./EditToolView";

afterEach(() => {
  cleanup();
});

describe("collectEditBlocks", () => {
  it("reads edits[] and falls back to historical top-level strings", () => {
    expect(
      collectEditBlocks({
        file_path: "a.rs",
        edits: [
          { old_string: "a", new_string: "b", replace_all: true },
          { old_string: "c", new_string: "d" },
        ],
      }),
    ).toEqual([
      { oldString: "a", newString: "b", replaceAll: true },
      { oldString: "c", newString: "d", replaceAll: false },
    ]);
    expect(
      collectEditBlocks({
        file_path: "a.rs",
        old_string: "foo",
        new_string: "bar",
      }),
    ).toEqual([{ oldString: "foo", newString: "bar", replaceAll: false }]);
  });
});

describe("EditToolView", () => {
  it("renders a single requested diff and the result body", () => {
    render(
      <EditToolView
        name="edit"
        status="ok"
        input={{
          file_path: "src/a.rs",
          edits: [{ old_string: "fn start() {}", new_string: "fn main() {}" }],
        }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output: "Edited src/a.rs",
        }}
      />,
    );
    expect(screen.getByText("src/a.rs")).toBeTruthy();
    expect(screen.getByText("1 edit")).toBeTruthy();
    expect(screen.getByText("fn start() {}")).toBeTruthy();
    expect(screen.getByText("fn main() {}")).toBeTruthy();
    expect(screen.getByText("Edited src/a.rs")).toBeTruthy();
  });

  it("labels each block as a request preview and shows replace_all", () => {
    render(
      <EditToolView
        name="edit"
        status="warning"
        input={{
          file_path: "src/a.rs",
          edits: [
            { old_string: "foo", new_string: "bar" },
            { old_string: "old_api(", new_string: "new_api(", replace_all: true },
          ],
        }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output:
            "Edited src/a.rs (1 applied / 0 warning / 1 failed). File updated.\n\nWarning: some edits were not applied",
        }}
      />,
    );
    expect(screen.getByText("2 edits")).toBeTruthy();
    expect(screen.getByText(/edit 1/)).toBeTruthy();
    expect(screen.getByText(/replace_all/)).toBeTruthy();
    expect(screen.getAllByText(/request preview/).length).toBeGreaterThan(0);
    expect(screen.getByText(/File updated/)).toBeTruthy();
  });

  it("still renders historical top-level old_string/new_string", () => {
    render(
      <EditToolView
        name="edit"
        status="ok"
        input={{
          file_path: "legacy.rs",
          old_string: "alpha",
          new_string: "beta",
          mode: "exact",
        }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output: "Edited legacy.rs",
        }}
      />,
    );
    expect(screen.getByText("legacy.rs")).toBeTruthy();
    expect(screen.getByText("alpha")).toBeTruthy();
    expect(screen.getByText("beta")).toBeTruthy();
  });

  it("keeps a failed edit body inline without a warning fold", () => {
    render(
      <EditToolView
        name="edit"
        status="failed"
        input={{
          file_path: "src/a.rs",
          edits: [{ old_string: "missing", new_string: "nope" }],
        }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output:
            "Error: No edits applied in src/a.rs (0 applied / 0 warning / 1 failed). File was not modified.\n\n[1] failed: no_useful_match\nNo sufficiently similar region was found.",
        }}
      />,
    );
    expect(screen.getByText(/No sufficiently similar region was found/)).toBeTruthy();
    expect(screen.queryByText("Tool warning")).toBeNull();
  });

  it("collapses Hint-only LSP notes after a successful edit", () => {
    render(
      <EditToolView
        name="edit"
        status="ok"
        input={{
          file_path: "src/a.rs",
          edits: [{ old_string: "fn start() {}", new_string: "fn main() {}" }],
        }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output: "Edited src/a.rs (1 applied / 0 warning / 0 failed). File updated.\n\nHint: LSP note — rust-analyzer",
        }}
      />,
    );
    expect(screen.getByText(/File updated/)).toBeTruthy();
    expect(screen.getByText("LSP note")).toBeTruthy();
  });
});
