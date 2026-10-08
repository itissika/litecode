import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { symbolMentionSource } from "../../lib/knowledge/markers";
import { LITECODE_PATHS_MIME, LITECODE_SPAN_MIME } from "../../lib/dropPayload";
import { useEditorStore } from "../../stores/editorStore";
import { fakeTransfer } from "../../test/fakeTransfer";
import { MentionEditor, type MentionEditorHandle } from "./MentionEditor";
import { clearSymbolCache } from "./suggestions";

const fetchMentionPaths = vi.hoisted(() => vi.fn());
const fetchSymbols = vi.hoisted(() => vi.fn());
const fetchSymbolAt = vi.hoisted(() => vi.fn());

vi.mock("../../api/workspace", () => ({
  fetchMentionPaths,
  fetchSymbols,
  fetchSymbolAt,
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

describe("MentionEditor popout document", () => {
  it("keeps the @ menu in the editor document after the panel moves", async () => {
    const onChange = vi.fn();
    const handle = editorHandle();
    render(
      <MentionEditor
        label="正文"
        value=""
        candidates={["alpha"]}
        onChange={onChange}
        handle={handle}
      />,
    );
    await waitFor(() => expect(handle.current).toBeTruthy());
    const field = document.querySelector<HTMLElement & { litecodeEditor?: { view: { root: Node } } }>(
      '[aria-label="正文"]',
    );
    const editor = field?.litecodeEditor;
    const shell = field?.parentElement;
    const home = shell?.parentElement;
    if (!editor || !shell || !home) throw new Error("editor missing");
    expect(editor.view.root).toBe(document);

    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    const pop = frame.contentDocument;
    if (!pop) throw new Error("popout document missing");
    try {
      pop.body.appendChild(shell);
      await Promise.resolve();

      expect(editor.view.root).toBe(pop);
      handle.current?.insertText("@al");
      await waitFor(() => {
        expect(pop.body.querySelector(".knowledge-mention-menu")?.textContent).toContain("alpha");
      });
      expect(document.querySelector(".knowledge-mention-menu")).toBeNull();
      const option = pop.body.querySelector<HTMLElement>('[role="option"]');
      if (!option) throw new Error("suggestion option missing");
      fireEvent.click(option);
      await waitFor(() => {
        const written = onChange.mock.calls.map((call) => String(call[0])).join("\n");
        expect(written).toContain("alpha");
      });
    } finally {
      // React unmount removes the shell from the document that rendered it.
      if (shell.ownerDocument !== document) home.appendChild(shell);
      frame.remove();
    }
  });
});

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
    const hashes = screen.getAllByRole("button", { name: "Pick a symbol in manager.rs" });
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
    const chip = await screen.findByRole("button", { name: "a.rs : fn save" });
    expect(chip.closest(".knowledge-token")?.classList.contains("is-symbol")).toBe(true);
  });
});

describe("MentionEditor drops", () => {
  function dropEvent(dt: DataTransfer): Event {
    const event = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "dataTransfer", { value: dt });
    Object.defineProperty(event, "clientX", { value: 8 });
    Object.defineProperty(event, "clientY", { value: 8 });
    return event;
  }

  async function dropOnEditor(dt: DataTransfer) {
    const onChange = vi.fn();
    render(
      <MentionEditor
        label="正文"
        value=""
        candidates={[]}
        onChange={onChange}
        onMentionDrop
      />,
    );
    const el = document.querySelector<HTMLElement>(".mention-editor-body");
    if (!el) throw new Error("editor missing");
    el.getBoundingClientRect = () =>
      ({
        x: 0,
        y: 0,
        left: 0,
        top: 0,
        right: 200,
        bottom: 40,
        width: 200,
        height: 40,
        toJSON: () => ({}),
      }) as DOMRect;
    document.elementFromPoint = () => el;
    el.dispatchEvent(dropEvent(dt));
    return onChange;
  }

  it("inserts a file chip for a tree path and does not open an absolute path", async () => {
    const openFile = vi.fn(async () => {});
    const previous = useEditorStore.getState().openFile;
    useEditorStore.setState({ openFile });

    const tree = fakeTransfer();
    tree.setData(LITECODE_PATHS_MIME, JSON.stringify(["src/a.rs"]));
    const onChange = await dropOnEditor(tree);
    const chip = await screen.findByRole("button", { name: ".../src/a.rs" });
    fireEvent.click(chip);
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    await waitFor(() => {
      const written = onChange.mock.calls.map((call) => String(call[0])).join("\n");
      expect(written).toContain('[@ file="src/a.rs"]');
    });

    cleanup();
    const outside = fakeTransfer();
    outside.setData("text/uri-list", "file:///C:/outside/a.ts");
    await dropOnEditor(outside);
    expect(await screen.findByText(".../outside/a.ts")).toBeTruthy();
    expect(screen.queryByRole("button", { name: ".../outside/a.ts" })).toBeNull();

    useEditorStore.setState({ openFile: previous });
  });

  it("inserts a symbol chip for a code span", async () => {
    fetchSymbolAt.mockResolvedValue({ chain: "fn save" });
    const span = fakeTransfer();
    span.setData(
      LITECODE_SPAN_MIME,
      JSON.stringify({ path: "src/a.rs", start: 4, end: 9 }),
    );
    span.setData("text/plain", "fn save() {}");
    const onChange = await dropOnEditor(span);
    expect(await screen.findByRole("button", { name: "a.rs : fn save" })).toBeTruthy();
    await waitFor(() => {
      const written = onChange.mock.calls.map((call) => String(call[0])).join("\n");
      expect(written).toContain('symbol="fn save"');
      expect(written).toContain('lines="4-9"');
    });
  });
});
