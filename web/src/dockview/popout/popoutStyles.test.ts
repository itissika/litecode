import { describe, expect, it } from "vitest";

import { mirrorDocumentStyles } from "./popoutStyles";

function scratchDoc(): Document {
  return document.implementation.createHTMLDocument("scratch");
}

function style(doc: Document, css: string): HTMLStyleElement {
  const el = doc.createElement("style");
  el.textContent = css;
  doc.head.appendChild(el);
  return el;
}

const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("mirrorDocumentStyles", () => {
  it("copies the opener sheets and drops dockview's rule copies", () => {
    const source = scratchDoc();
    style(source, ".main{color:red}");
    const link = source.createElement("link");
    link.rel = "stylesheet";
    link.href = "http://127.0.0.1:1/app.css";
    source.head.appendChild(link);

    const target = scratchDoc();
    const stale = target.createElement("style");
    stale.textContent = ".main{color:red}";
    target.head.appendChild(stale);

    const stop = mirrorDocumentStyles(target, source);
    expect(target.head.querySelectorAll("style").length).toBe(1);
    expect(target.head.querySelector<HTMLStyleElement>("style")?.textContent).toBe(
      ".main{color:red}",
    );
    expect(target.head.querySelector("link")?.getAttribute("href")).toBe(
      "http://127.0.0.1:1/app.css",
    );
    stop();
  });

  it("carries a sheet injected after the popout opened", async () => {
    const source = scratchDoc();
    const target = scratchDoc();
    const stop = mirrorDocumentStyles(target, source);
    expect(target.head.querySelectorAll("style").length).toBe(0);

    style(source, ".mtk1{color:#d4d4d4}");
    await nextTask();
    expect(target.head.querySelector<HTMLStyleElement>("style")?.textContent).toBe(
      ".mtk1{color:#d4d4d4}",
    );
    stop();
  });

  it("follows a theme rewrite of an existing sheet", async () => {
    const source = scratchDoc();
    const sheet = style(source, ".mtk1{color:#111}");
    const target = scratchDoc();
    const stop = mirrorDocumentStyles(target, source);

    sheet.textContent = ".mtk1{color:#eee}";
    await nextTask();
    expect(target.head.querySelector<HTMLStyleElement>("style")?.textContent).toBe(
      ".mtk1{color:#eee}",
    );

    const appended = source.createElement("style");
    appended.textContent = ".extra{}";
    sheet.appendChild(appended);
    await nextTask();
    expect(target.head.querySelector<HTMLStyleElement>("style")?.textContent).toBe(
      ".mtk1{color:#eee}.extra{}",
    );
    stop();
  });

  it("removes a sheet the opener removed and cleans up on stop", async () => {
    const source = scratchDoc();
    const sheet = style(source, ".a{}");
    const target = scratchDoc();
    const stop = mirrorDocumentStyles(target, source);

    sheet.remove();
    await nextTask();
    expect(target.head.querySelectorAll("style").length).toBe(0);

    style(source, ".b{}");
    await nextTask();
    expect(target.head.querySelectorAll("style").length).toBe(1);
    stop();
    expect(target.head.querySelectorAll("style").length).toBe(0);
  });
});
