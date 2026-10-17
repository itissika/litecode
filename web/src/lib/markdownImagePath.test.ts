import { describe, expect, it } from "vitest";

import { resolveMarkdownImage } from "./markdownImagePath";

describe("resolveMarkdownImage", () => {
  it("joins a sibling, a ./ path, and a .. path onto the markdown file", () => {
    expect(resolveMarkdownImage("docs/readme.md", "shot.gif")).toEqual({
      kind: "workspace",
      path: "docs/shot.gif",
    });
    expect(resolveMarkdownImage("docs/readme.md", "./shot.gif")).toEqual({
      kind: "workspace",
      path: "docs/shot.gif",
    });
    expect(resolveMarkdownImage("docs/guide/readme.md", "../img/a.png")).toEqual({
      kind: "workspace",
      path: "docs/img/a.png",
    });
  });

  it("keeps an https image and drops other schemes", () => {
    expect(resolveMarkdownImage("docs/readme.md", "https://example.com/a.gif")).toEqual({
      kind: "remote",
      url: "https://example.com/a.gif",
    });
    expect(resolveMarkdownImage("docs/readme.md", "http://example.com/a.png")).toEqual({
      kind: "remote",
      url: "http://example.com/a.png",
    });
    expect(resolveMarkdownImage("docs/readme.md", "file:docs/shot.gif")).toBeNull();
    expect(resolveMarkdownImage("docs/readme.md", "")).toBeNull();
    expect(resolveMarkdownImage("docs/readme.md", undefined)).toBeNull();
  });

  it("drops a path that leaves the workspace or is not an image", () => {
    expect(resolveMarkdownImage("readme.md", "../secret.gif")).toBeNull();
    expect(resolveMarkdownImage("docs/readme.md", "./notes.md")).toBeNull();
    expect(resolveMarkdownImage("docs/readme.md", "/abs/shot.gif")).toBeNull();
  });

  it("strips a query and hash before joining", () => {
    expect(resolveMarkdownImage("docs/readme.md", "./shot.gif?raw=1#fig")).toEqual({
      kind: "workspace",
      path: "docs/shot.gif",
    });
  });
});
