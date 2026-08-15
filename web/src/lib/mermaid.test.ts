import { describe, expect, it } from "vitest";

import { isMermaidLang, withMindmapTreeLayout } from "./mermaid";

describe("mermaid lang detection", () => {
  it("recognizes mermaid code fence languages", () => {
    expect(isMermaidLang("mermaid")).toBe(true);
    expect(isMermaidLang("mmd")).toBe(true);
    expect(isMermaidLang("Mermaid")).toBe(true);
  });

  it("rejects other languages", () => {
    expect(isMermaidLang("rust")).toBe(false);
    expect(isMermaidLang("typescript")).toBe(false);
  });
});

describe("withMindmapTreeLayout", () => {
  it("injects dagre layout into a plain mindmap", () => {
    const code = "mindmap\n  root((Root))\n    A\n    B";
    expect(withMindmapTreeLayout(code)).toBe(
      "---\nconfig:\n  layout: dagre\n---\n" + code,
    );
  });

  it("is case-insensitive on the mindmap keyword", () => {
    const code = "MindMap\n  root((Root))";
    expect(withMindmapTreeLayout(code).startsWith("---")).toBe(true);
  });

  it("leaves non-mindmap diagrams untouched", () => {
    const code = "graph TD\n  A --> B";
    expect(withMindmapTreeLayout(code)).toBe(code);
  });

  it("leaves frontmatter-bearing mindmaps untouched (author intent wins)", () => {
    const code = "---\nconfig:\n  layout: something\n---\nmindmap\n  root((Root))";
    expect(withMindmapTreeLayout(code)).toBe(code);
  });
});
