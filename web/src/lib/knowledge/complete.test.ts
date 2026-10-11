import { describe, expect, it } from "vitest";

import {
  renderKnowledgeMarkdown,
  replaceKnowledgeKey,
} from "./document";
import { mentionSource } from "./markers";

describe("replaceKnowledgeKey", () => {
  it("rewrites a matching key and leaves a different key", () => {
    const markdown = renderKnowledgeMarkdown({
      key: "session",
      status: "enabled",
      summary: "",
      body: `见 ${mentionSource("seq")} 与 ${mentionSource("other")}。`,
    });
    const next = replaceKnowledgeKey(markdown, "seq", "sequence");
    expect(next).toContain("node : session");
    expect(next).toContain(mentionSource("sequence"));
    expect(next).toContain(mentionSource("other"));
    expect(next).not.toContain('key="seq"');
  });
});
