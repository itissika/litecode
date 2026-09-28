import { describe, expect, it } from "vitest";

import {
  parseKnowledgeMarkdown,
  renderKnowledgeMarkdown,
  replaceKnowledgeKey,
  upgradeKnowledgeMarkdown,
} from "./document";
import { mentionSource } from "./markers";

describe("upgradeKnowledgeMarkdown", () => {
  it("rewrites a legacy marker, drops ref lines, and fills a missing summary", () => {
    const markdown = [
      "```node",
      "node : session",
      "status : enabled",
      "ref : unused",
      "```",
      "",
      "见 [[node : seq]]。",
      "",
    ].join("\n");
    const upgraded = upgradeKnowledgeMarkdown(markdown);
    const parsed = parseKnowledgeMarkdown("session.md", upgraded ?? "");
    expect(parsed.summary).toBe("见 seq。");
    expect(parsed.refs).toEqual(["seq"]);
    expect(parsed.body).toContain(mentionSource("seq"));
    expect(parsed.hadRefLine).toBe(false);
    expect(upgraded).not.toContain("ref :");
    expect(upgraded).not.toContain("[[node");
    expect(upgradeKnowledgeMarkdown(upgraded ?? "")).toBeNull();
  });
});

describe("replaceKnowledgeKey", () => {
  it("rewrites the id and a matching label, and leaves the declaration", () => {
    const markdown = renderKnowledgeMarkdown({
      key: "session",
      status: "enabled",
      summary: "",
      body: `见 ${mentionSource("seq")} 与 ${mentionSource("other", "seq")}。`,
    });
    const next = replaceKnowledgeKey(markdown, "seq", "sequence");
    expect(next).toContain("node : session");
    expect(next).toContain(mentionSource("sequence"));
    expect(next).toContain(mentionSource("other", "sequence"));
    expect(next).not.toContain('id="seq"');
  });
});
