import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { symbolMentionSource } from "../../lib/knowledge/markers";
import { MentionEditor, type MentionEditorHandle } from "./MentionEditor";
import { clearSymbolCache } from "./suggestions";

const fetchMentionPaths = vi.hoisted(() => vi.fn());
const fetchSymbols = vi.hoisted(() => vi.fn());

vi.mock("../../api/workspace", () => ({
  fetchMentionPaths,
  fetchSymbols,
}));

const files = [
  { path: "src/a/manager.rs", file: true },
  { path: "src/b/manager.rs", file: true },
];

const symbols = [
  {
    chain: "fn test_run",
    kind: "function",
    name: "test_run",
    start_line: 4,
    end_line: 9,
    summary: "fn test_run()\n    assert!(true);",
  },
];

beforeEach(() => {
  clearSymbolCache();
  fetchMentionPaths.mockReset();
  fetchSymbols.mockReset();
  fetchMentionPaths.mockResolvedValue(files);
  fetchSymbols.mockResolvedValue(symbols);
});

afterEach(() => {
  cleanup();
});

function editorHandle() {
  return createRef<MentionEditorHandle>();
}

async function typeQuery(handle: { current: MentionEditorHandle | null }, text: string) {
  await waitFor(() => expect(handle.current).toBeTruthy());
  handle.current?.insertText(text);
}

function press(key: string) {
  const field = document.querySelector<HTMLElement>('[aria-label="正文"]');
  if (!field) throw new Error("editor missing");
  field.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

describe("MentionEditor symbol mode", () => {
  it("locks the highlighted file and inserts a symbol shortcode without lines", async () => {
    const onChange = vi.fn();
    const handle = editorHandle();
    render(
      <MentionEditor
        label="正文"
        value=""
        candidates={[]}
        symbolLines={false}
        onChange={onChange}
        handle={handle}
      />,
    );
    await typeQuery(handle, "/mang");
    expect(await screen.findByText("src/a/manager.rs")).toBeTruthy();
    const hashes = screen.getAllByRole("button", { name: "在 manager.rs 中选符号" });
    fireEvent.click(hashes[0]);
    expect(fetchSymbols).toHaveBeenCalledWith("src/a/manager.rs");
    const row = await screen.findByRole("option", { name: /fn test_run/ });
    fireEvent.click(row);
    const written = onChange.mock.calls.map((call) => String(call[0])).join("\n");
    expect(written).toContain(
      symbolMentionSource("src/a/manager.rs", {
        symbol: "fn test_run",
        label: "manager.rs fn test_run",
      }),
    );
    expect(written).not.toContain('lines="');
  });

  it("uses the highlighted row when two files share a name and keeps lines in chat", async () => {
    const onChange = vi.fn();
    const handle = editorHandle();
    render(
      <MentionEditor
        label="正文"
        value=""
        candidates={[]}
        symbolLines
        onChange={onChange}
        handle={handle}
      />,
    );
    await typeQuery(handle, "/mang");
    await screen.findByText("src/b/manager.rs");
    press("ArrowDown");
    press("#");
    expect(fetchSymbols).toHaveBeenCalledWith("src/b/manager.rs");
    const row = await screen.findByRole("option", { name: /fn test_run/ });
    fireEvent.click(row);
    const written = onChange.mock.calls.map((call) => String(call[0])).join("\n");
    expect(written).toContain(
      symbolMentionSource("src/b/manager.rs", {
        symbol: "fn test_run",
        lines: "4-9",
        label: "manager.rs fn test_run",
      }),
    );
  });

  it("renders a stored symbol shortcode as a cyan capsule", async () => {
    render(
      <MentionEditor
        label="正文"
        value={symbolMentionSource("src/a.rs", { symbol: "fn save", label: "fn save" })}
        candidates={[]}
        onChange={() => undefined}
      />,
    );
    const chip = await screen.findByRole("button", { name: "a.rs fn save" });
    expect(chip.closest(".knowledge-token")?.classList.contains("is-symbol")).toBe(true);
  });
});
