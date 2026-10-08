import { Editor } from "@tiptap/core";
import Document from "@tiptap/extension-document";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import { afterEach, describe, expect, it } from "vitest";

import { trackEditorView } from "./editorRoot";

describe("trackEditorView", () => {
  const cleanups: Array<() => void> = [];

  afterEach(() => {
    for (const cleanup of cleanups.splice(0)) cleanup();
  });

  it("follows the editor into another document and back", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    const pop = frame.contentDocument;
    if (!pop) throw new Error("popout document missing");

    const editor = new Editor({
      element: host,
      extensions: [Document, Paragraph, Text],
      content: "hi",
    });
    const stop = trackEditorView(editor.view);
    cleanups.push(() => {
      stop();
      editor.destroy();
      host.remove();
      frame.remove();
    });

    expect(editor.view.root).toBe(document);
    const selectionEvents: string[] = [];
    const add = pop.addEventListener.bind(pop);
    pop.addEventListener = ((type: string, listener: EventListener, options?: boolean | AddEventListenerOptions) => {
      selectionEvents.push(type);
      add(type, listener, options);
    }) as typeof pop.addEventListener;
    pop.body.appendChild(host);
    await Promise.resolve();
    expect(editor.view.root).toBe(pop);
    expect(selectionEvents).toContain("selectionchange");

    document.body.appendChild(host);
    await Promise.resolve();
    expect(editor.view.root).toBe(document);
  });
});
