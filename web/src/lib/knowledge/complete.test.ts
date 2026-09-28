import { describe, expect, it } from "vitest";

import { applyCompletion, completionAt } from "./complete";
import {
  parseKnowledgeMarkdown,
  renderKnowledgeMarkdown,
  replaceKnowledgeKey,
  upgradeKnowledgeMarkdown,
} from "./document";

describe("upgradeKnowledgeMarkdown", () => {
  it("adds summary and ref lines when the fence has neither", () => {
    const markdown = [
      "```node",
      "node : session",
      "status : enabled",
      "```",
      "",
      "见 [[node : seq]]。",
      "",
    ].join("\n");
    const upgraded = upgradeKnowledgeMarkdown(markdown);
    const parsed = parseKnowledgeMarkdown("session.md", upgraded ?? "");
    expect(parsed.summary).toBe("见 seq。");
    expect(parsed.refs).toEqual(["seq"]);
    expect(upgradeKnowledgeMarkdown(upgraded ?? "")).toBeNull();
  });
});

describe("replaceKnowledgeKey", () => {
  it("rewrites markers and leaves the declaration line", () => {
    const markdown = renderKnowledgeMarkdown({
      key: "session",
      status: "enabled",
      summary: "",
      refs: ["seq"],
      body: "见 [[node:seq]]。",
    });
    const next = replaceKnowledgeKey(markdown, "seq", "sequence");
    expect(next).toContain("node : session");
    expect(next).toContain("[[node : sequence]]");
    expect(next).not.toContain("[[node:seq]]");
  });
});

describe("completionAt", () => {
  it("completes a ref line and a body marker from the given keys", () => {
    const ref = completionAt("refs", "ref : se", 8, ["seq", "session"]);
    expect(ref?.items).toEqual(["seq", "session"]);
    const applied = applyCompletion("refs", "ref : se", ref!, "seq");
    expect(applied.text).toBe("ref : seq");

    const body = completionAt("body", "见 [[node : se", 13, ["seq"]);
    expect(body?.items).toEqual(["seq"]);
    expect(applyCompletion("body", "见 [[node : se", body!, "seq").text).toBe(
      "见 [[node : seq]]",
    );
  });

  it("ignores a bare double bracket", () => {
    expect(completionAt("body", "[[se", 4, ["seq"])).toBeNull();
  });
});
