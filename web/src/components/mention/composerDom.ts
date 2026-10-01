import { act } from "@testing-library/react";
import type { Editor } from "@tiptap/core";

import { bodyToContent } from "./serialize";

type EditorHost = HTMLElement & { litecodeEditor?: Editor };

export function composerEditor(label: string): Editor {
  const dom = document.querySelector<EditorHost>(`[aria-label="${label}"]`);
  const editor = dom?.litecodeEditor;
  if (!editor) throw new Error(`composer "${label}" is not mounted`);
  return editor;
}

export function composerText(label: string): string {
  return composerEditor(label).getText({ blockSeparator: "\n" });
}

export function setComposerText(label: string, text: string) {
  act(() => {
    composerEditor(label).commands.setContent(bodyToContent(text));
  });
}

export function pressComposerKey(label: string, key: string, shiftKey = false) {
  const dom = document.querySelector<HTMLElement>(`[aria-label="${label}"]`);
  if (!dom) throw new Error(`composer "${label}" is not mounted`);
  act(() => {
    dom.dispatchEvent(
      new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }),
    );
  });
}
