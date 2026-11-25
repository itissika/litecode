import { afterEach, describe, expect, it, vi } from "vitest";

import { installCitationCopy, onCitationCopy } from "./citationCopy";

function clipboard() {
  const store = new Map<string, string>();
  return {
    store,
    data: {
      setData: (type: string, value: string) => void store.set(type, value),
    } as unknown as DataTransfer,
  };
}

/** Select everything under `html`, the way a drag across a reply does. */
function selectAll(html: string, className = "agent-markdown"): HTMLElement {
  const body = document.createElement("div");
  body.className = className;
  body.innerHTML = html;
  document.body.appendChild(body);
  const range = document.createRange();
  range.selectNodeContents(body);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
  return body;
}

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  document.body.innerHTML = "";
});

describe("onCitationCopy", () => {
  it("copies a file range chip as its standard Markdown link, not its label", () => {
    const prose = selectAll(
      'see <span class="knowledge-token" data-citation="[a.ts:12-30](src/a.ts#L12-L30)">a.ts : 12-30</span> here',
    );
    const { store, data } = clipboard();
    const preventDefault = vi.fn();

    onCitationCopy({ clipboardData: data, target: prose, preventDefault });

    const text = store.get("text/plain") ?? "";
    expect(text).toContain("[a.ts:12-30](src/a.ts#L12-L30)");
    expect(text).not.toContain("a.ts : 12-30");
    expect(store.get("text/html") ?? "").toContain(
      "[a.ts:12-30](src/a.ts#L12-L30)",
    );
    expect(preventDefault).toHaveBeenCalled();
  });

  it("copies node and symbol chips in the same selection", () => {
    const prose = selectAll(
      '<span class="knowledge-token" data-citation="[seq](knowledge/arch/seq.md)">seq</span>' +
        '<span class="knowledge-token" data-citation="[Store.save](src/a.rs#Store.save)">a.rs : save</span>',
    );
    const { store, data } = clipboard();

    onCitationCopy({ clipboardData: data, target: prose, preventDefault: vi.fn() });

    const text = store.get("text/plain") ?? "";
    expect(text).toContain("[seq](knowledge/arch/seq.md)");
    expect(text).toContain("[Store.save](src/a.rs#Store.save)");
  });

  it("leaves a selection without chips to the browser", () => {
    const prose = selectAll("plain reply with <span>no chip</span>");
    const { store, data } = clipboard();
    const preventDefault = vi.fn();

    onCitationCopy({ clipboardData: data, target: prose, preventDefault });

    expect(store.size).toBe(0);
    expect(preventDefault).not.toHaveBeenCalled();
  });

  it("leaves the editor alone: a mention there is a node, not text", () => {
    const editor = selectAll(
      '<span class="knowledge-token" data-citation="[a.ts](src/a.ts)">a.ts</span>',
      "agent-markdown",
    );
    editor.setAttribute("contenteditable", "true");
    const { store, data } = clipboard();
    const preventDefault = vi.fn();

    onCitationCopy({ clipboardData: data, target: editor, preventDefault });

    expect(store.size).toBe(0);
    expect(preventDefault).not.toHaveBeenCalled();
  });

  it("rewrites a chip wherever the selection starts, even off the prose", () => {
    const prose = selectAll(
      '<span class="knowledge-token" data-citation="[a.ts](src/a.ts)">a.ts</span>',
    );
    const range = document.createRange();
    range.selectNodeContents(document.body);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    const { store, data } = clipboard();
    const preventDefault = vi.fn();

    onCitationCopy({ clipboardData: data, target: document.body, preventDefault });

    expect(prose).toBeTruthy();
    expect(store.get("text/plain")).toContain("[a.ts](src/a.ts)");
    expect(preventDefault).toHaveBeenCalled();
  });
});

describe("installCitationCopy", () => {
  function copyEvent(target: Element, data: DataTransfer): Event {
    const event = new Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", { value: data });
    target.dispatchEvent(event);
    return event;
  }

  it("catches a copy aimed anywhere in the document and rewrites the chips", () => {
    const stop = installCitationCopy(document);
    try {
      const prose = selectAll(
        '<span class="knowledge-token" data-citation="[seq](knowledge/arch/seq.md)">seq</span>',
      );
      const { store, data } = clipboard();

      const event = copyEvent(document.body, data);

      expect(event.defaultPrevented).toBe(true);
      expect(store.get("text/plain")).toContain("[seq](knowledge/arch/seq.md)");
      expect(prose).toBeTruthy();
    } finally {
      stop();
    }
  });

  it("can be removed again", () => {
    installCitationCopy(document)();
    selectAll(
      '<span class="knowledge-token" data-citation="[seq](knowledge/arch/seq.md)">seq</span>',
    );
    const { store, data } = clipboard();

    const event = copyEvent(document.body, data);

    expect(event.defaultPrevented).toBe(false);
    expect(store.size).toBe(0);
  });
});
