import { describe, expect, it } from "vitest";

import { extractMarkers, knowledgePreview, splitKnowledgeRefs } from "./markers";
import { remarkKnowledgeRef } from "./remarkKnowledgeRef";

describe("splitKnowledgeRefs", () => {
  it("splits markers out of surrounding text and trims the key", () => {
    expect(splitKnowledgeRefs("see [[ seq ]] now")).toEqual([
      { type: "text", value: "see " },
      { type: "ref", key: "seq" },
      { type: "text", value: " now" },
    ]);
  });

  it("leaves a whitespace-only marker as text", () => {
    expect(splitKnowledgeRefs("[[ ]]")).toEqual([
      { type: "text", value: "[[ ]]" },
    ]);
  });
});

describe("extractMarkers", () => {
  it("reads every prose marker", () => {
    expect(extractMarkers("[[session]] and [[seq]]")).toEqual([
      "session",
      "seq",
    ]);
  });

  it("ignores fenced blocks, tilde fences, and inline code", () => {
    const markdown = [
      "keep [[session]]",
      "```ts",
      "[[seq]]",
      "```",
      "also `[[revert]]` stays code",
      "~~~",
      "[[compact]]",
      "~~~",
      "end [[item]]",
    ].join("\n");
    expect(extractMarkers(markdown)).toEqual(["session", "item"]);
  });
});

describe("knowledgePreview", () => {
  it("drops fences and unwraps markers", () => {
    const value = "alpha [[seq]]\n\n```\n[[hidden]]\n```\nbeta";
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
          children: [{ type: "text", value: "see [[seq]]" }],
        },
        { type: "code", value: "[[seq]]" },
        {
          type: "paragraph",
          children: [{ type: "inlineCode", value: "[[revert]]" }],
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
    expect(tree.children[1]).toEqual({ type: "code", value: "[[seq]]" });
    expect(tree.children[2]?.children?.[0]).toEqual({
      type: "inlineCode",
      value: "[[revert]]",
    });
  });
});
