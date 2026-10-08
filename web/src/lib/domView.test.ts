import { describe, expect, it } from "vitest";

import { hostElementFromTarget, viewOf } from "./domView";

describe("viewOf", () => {
  it("uses the window that owns the node", () => {
    const iframe = document.createElement("iframe");
    document.body.appendChild(iframe);
    const childDoc = iframe.contentDocument;
    const childView = iframe.contentWindow;
    expect(childDoc).toBeTruthy();
    expect(childView).toBeTruthy();
    const node = childDoc!.createElement("div");
    childDoc!.body.appendChild(node);
    expect(viewOf(node)).toBe(childView);
    iframe.remove();
  });

  it("falls back to the opener when the node has no window", () => {
    expect(viewOf(null)).toBe(window);
  });
});

describe("hostElementFromTarget", () => {
  it("reads a text hit inside the window that owns the scope", () => {
    const iframe = document.createElement("iframe");
    document.body.appendChild(iframe);
    const doc = iframe.contentDocument;
    const view = iframe.contentWindow;
    if (!doc || !view) throw new Error("popout document missing");
    const form = doc.createElement("div");
    form.setAttribute("data-mini-chat-input", "");
    const text = doc.createTextNode("hello");
    form.appendChild(text);
    doc.body.appendChild(form);

    const hit = hostElementFromTarget(text, form);
    expect(hit).toBe(form);
    expect(hostElementFromTarget(document.body, form)).toBeNull();
    iframe.remove();
  });

  it("accepts a text node built in the opener and moved into the popout", () => {
    const iframe = document.createElement("iframe");
    document.body.appendChild(iframe);
    const doc = iframe.contentDocument;
    if (!doc) throw new Error("popout document missing");
    const form = document.createElement("div");
    form.setAttribute("data-mini-chat-input", "");
    const editor = document.createElement("div");
    editor.textContent = "hello";
    form.appendChild(editor);
    doc.body.appendChild(form);
    const text = editor.firstChild;
    if (!text) throw new Error("text missing");

    const hit = hostElementFromTarget(text, form);
    expect(hit?.closest("[data-mini-chat-input]")).toBe(form);
    iframe.remove();
  });
});
