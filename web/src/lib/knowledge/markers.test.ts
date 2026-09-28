import { describe, expect, it } from "vitest";

import { extractMarkers, knowledgePreview, mentionSource, splitKnowledgeRefs } from "./markers";
import { remarkKnowledgeRef } from "./remarkKnowledgeRef";

const seq = mentionSource("seq");

describe("splitKnowledgeRefs", () => {
  it("splits a mention and keeps id and label", () => {
    expect(splitKnowledgeRefs(`see ${mentionSource("seq", "序号")} now`)).toEqual([
      { type: "text", value: "see " },
      { type: "ref", id: "seq", label: "序号" },
      { type: "text", value: " now" },
    ]);
  });

  it("leaves plain @text and old brackets as text", () => {
    expect(splitKnowledgeRefs("@seq and [[node : seq]] and [[seq]]")).toEqual([
      { type: "text", value: "@seq and [[node : seq]] and [[seq]]" },
    ]);
  });
});

describe("extractMarkers", () => {
  it("reads every prose mention once, in order", () => {
    expect(
      extractMarkers(`${mentionSource("session")} and ${mentionSource("seq")} and ${seq}`),
    ).toEqual(["session", "seq"]);
  });

  it("ignores fenced blocks, tilde fences, inline code, and plain text", () => {
    const markdown = [
      `keep ${mentionSource("session")}`,
      "```ts",
      seq,
      "```",
      `also \`${mentionSource("revert")}\` stays code`,
      "~~~",
      mentionSource("compact"),
      "~~~",
      `end ${mentionSource("item")}`,
      "not @providers",
    ].join("\n");
    expect(extractMarkers(markdown)).toEqual(["session", "item"]);
  });
});

describe("knowledgePreview", () => {
  it("drops fences and shows the label", () => {
    const value = `alpha ${mentionSource("seq", "序号")}\n\n\`\`\`\n${seq}\n\`\`\`\nbeta`;
    expect(knowledgePreview(value, 2)).toBe("alpha 序号\nbeta");
  });
});

describe("remarkKnowledgeRef", () => {
  it("turns prose mentions into knowledge links and leaves code nodes", () => {
    const tree = {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [{ type: "text", value: `see ${seq}` }],
        },
        { type: "code", value: seq },
        {
          type: "paragraph",
          children: [{ type: "inlineCode", value: mentionSource("revert") }],
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
    expect(tree.children[1]).toEqual({ type: "code", value: seq });
    expect(tree.children[2]?.children?.[0]).toEqual({
      type: "inlineCode",
      value: mentionSource("revert"),
    });
  });
});
