import { describe, expect, it } from "vitest";

import { extractMarkers, knowledgePreview, splitKnowledgeRefs } from "./markers";
import { remarkKnowledgeRef } from "./remarkKnowledgeRef";

describe("splitKnowledgeRefs", () => {
  it("splits a node citation and keeps the key", () => {
    expect(splitKnowledgeRefs("see [[ node : seq ]] now")).toEqual([
      { type: "text", value: "see " },
      { type: "ref", key: "seq" },
      { type: "text", value: " now" },
    ]);
  });

  it("accepts a tight colon", () => {
    expect(splitKnowledgeRefs("[[node:seq]]")).toEqual([
      { type: "ref", key: "seq" },
    ]);
  });

  it("leaves bare double brackets as text", () => {
    expect(splitKnowledgeRefs("[[seq]] and [[ ]]")).toEqual([
      { type: "text", value: "[[seq]] and [[ ]]" },
    ]);
  });
});

describe("extractMarkers", () => {
  it("reads every prose marker", () => {
    expect(extractMarkers("[[node : session]] and [[node : seq]]")).toEqual([
      "session",
      "seq",
    ]);
  });

  it("ignores fenced blocks, tilde fences, inline code, and bare brackets", () => {
    const markdown = [
      "keep [[node : session]]",
      "```ts",
      "[[node : seq]]",
      "```",
      "also `[[node : revert]]` stays code",
      "~~~",
      "[[node : compact]]",
      "~~~",
      "end [[node : item]]",
      "not [[providers]]",
    ].join("\n");
    expect(extractMarkers(markdown)).toEqual(["session", "item"]);
  });
});

describe("knowledgePreview", () => {
  it("drops fences and unwraps markers", () => {
    const value = "alpha [[node : seq]]\n\n```\n[[node : hidden]]\n```\nbeta";
    expect(knowledgePreview(value, 2)).toBe("alpha seq\nbeta");
  });
});

describe("remarkKnowledgeRef", () => {
  it("turns prose markers into knowledge links and leaves code nodes", () => {
    const tree = {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [{ type: "text", value: "see [[node : seq]]" }],
        },
        { type: "code", value: "[[node : seq]]" },
        {
          type: "paragraph",
          children: [{ type: "inlineCode", value: "[[node : revert]]" }],
        },
      ],
    };
    remarkKnowledgeRef()(tree);
    expect(tree.children[0]?.children).toEqual([
      { type: "text", value: "see " },
      {
        type: "link",
        url: "knowledge:seq",
        children: [{ type: "text", value: "seq" }],
      },
    ]);
    expect(tree.children[1]).toEqual({ type: "code", value: "[[node : seq]]" });
    expect(tree.children[2]?.children?.[0]).toEqual({
      type: "inlineCode",
      value: "[[node : revert]]",
    });
  });
});
