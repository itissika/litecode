import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { LspNoteTail, splitLspNote, ToolResultBlock } from "./LspNote";

afterEach(() => {
  cleanup();
});

describe("splitLspNote", () => {
  it("splits at Warning, Hint, and Error tails", () => {
    expect(splitLspNote("Edited a.rs\n\nWarning: some edits were not applied")).toEqual({
      body: "Edited a.rs",
      lsp: "Warning: some edits were not applied",
    });
    expect(splitLspNote("Edited a.rs\n\nHint: LSP note — rust-analyzer")).toEqual({
      body: "Edited a.rs",
      lsp: "Hint: LSP note — rust-analyzer",
    });
    expect(splitLspNote("Edited a.rs\n\nError: extra note")).toEqual({
      body: "Edited a.rs",
      lsp: "Error: extra note",
    });
  });

  it("keeps the first signal when Warning precedes Hint", () => {
    const text =
      "Edited a.rs\n\nWarning: some edits were not applied\n\nHint: LSP note — errors\nError: missing ;";
    const { body, lsp } = splitLspNote(text);
    expect(body).toBe("Edited a.rs");
    expect(lsp).toContain("Warning: some edits were not applied");
    expect(lsp).toContain("Hint: LSP note");
  });

  it("does not split when Error is only a prefix", () => {
    expect(splitLspNote("Error: No edits applied in a.rs")).toEqual({
      body: "Error: No edits applied in a.rs",
      lsp: undefined,
    });
  });
});

describe("ToolResultBlock", () => {
  it("returns nothing for empty output", () => {
    const { container } = render(<ToolResultBlock />);
    expect(container.textContent).toBe("");
  });

  it("keeps multiline edit details inline before the warning fold", () => {
    render(
      <ToolResultBlock
        output={{
          type: "function_call_output",
          call_id: "c1",
          output:
            "Edited src/a.rs (1 applied / 0 warning / 1 failed). File updated.\n\n[1] applied: exact, 1 replacement (line 1)\n\n[2] failed: no_useful_match\nNo sufficiently similar region was found.\n\nWarning: some edits were not applied",
        }}
      />,
    );
    expect(screen.getByText(/File updated/)).toBeTruthy();
    expect(screen.getByText(/\[2\] failed: no_useful_match/)).toBeTruthy();
    expect(screen.getByText("Tool warning")).toBeTruthy();
  });
});

describe("LspNoteTail", () => {
  it("labels Error, Hint, and Warning tails", () => {
    const { rerender } = render(<LspNoteTail text="Error: boom" />);
    expect(screen.getByText("Tool error note")).toBeTruthy();
    rerender(<LspNoteTail text="Hint: LSP note — rust-analyzer" />);
    expect(screen.getByText("LSP note")).toBeTruthy();
    rerender(<LspNoteTail text="Warning: some edits were not applied" />);
    expect(screen.getByText("Tool warning")).toBeTruthy();
  });
});
