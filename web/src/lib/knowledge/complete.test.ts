import { describe, expect, it } from "vitest";

import {
  renderKnowledgeMarkdown,
  replaceKnowledgePath,
} from "./document";

describe("replaceKnowledgePath", () => {
  it("rewrites a matching knowledge path and leaves other links alone", () => {
    const markdown = renderKnowledgeMarkdown({
      key: "session",
      status: "enabled",
      summary: "",
      body: "见 [seq](knowledge/arch/seq.md) 与 [other](knowledge/arch/other.md)。",
    });
    const next = replaceKnowledgePath(
      markdown,
      "knowledge/arch/seq.md",
      "knowledge/arch/sequence.md",
    );
    expect(next).toContain("node : session");
    expect(next).toContain("[sequence](knowledge/arch/sequence.md)");
    expect(next).toContain("[other](knowledge/arch/other.md)");
    expect(next).not.toContain("knowledge/arch/seq.md");
  });
});
