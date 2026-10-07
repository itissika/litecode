import { describe, expect, it } from "vitest";

import { preparePopoutDocument } from "./popoutChrome";

describe("preparePopoutDocument", () => {
  it("uses the main dock theme class and leaves room for the title bar", () => {
    const doc = document.implementation.createHTMLDocument("Litecode");
    doc.body.innerHTML = '<div id="dv-popout-window"></div>';
    preparePopoutDocument(doc, "light");
    expect(doc.documentElement.getAttribute("data-dv-theme")).toBe("light");
    expect(doc.documentElement.classList.contains("litecode-dv-base")).toBe(true);
    const shell = doc.getElementById("dv-popout-window") as HTMLElement;
    expect(shell.style.top).toBe("32px");
    expect(shell.style.height).toBe("calc(100% - 32px)");
  });
});
