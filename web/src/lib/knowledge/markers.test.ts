import { describe, expect, it } from "vitest";

import {
  extractFileRefs,
  extractMarkers,
  fileMentionSource,
  isWorkspaceFileRef,
  symbolMentionSource,
  knowledgePreview,
  mentionSource,
  splitBodyRefs,
  splitKnowledgeRefs,
} from "./markers";
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

  it("keeps inline code and still ignores a citation inside it or a fence", () => {
    const hidden = mentionSource("hidden");
    const value = `see \`node : seq\` and ${mentionSource("seq", "序号")}\n\`\`\`\n${hidden}\n\`\`\`\n`;
    expect(knowledgePreview(value, 3)).toBe("see `node : seq` and 序号");
    expect(extractMarkers(value)).toEqual(["seq"]);
    expect(extractMarkers(`\`${hidden}\``)).toEqual([]);
    expect(knowledgePreview(`\`${hidden}\``, 1)).toContain('id="hidden"');
  });

  it("clips inline code past 24 characters", () => {
    const long = "a".repeat(30);
    expect(knowledgePreview(`\`${long}\``, 1)).toBe(`\`${"a".repeat(24)}…\``);
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

describe("file citations", () => {
  it("keeps a file citation out of node markers and out of code", () => {
    const file = fileMentionSource("src/a.rs");
    const markdown = [
      `see ${file} and ${mentionSource("seq")}`,
      "```",
      fileMentionSource("skip.rs"),
      "```",
      `\`${fileMentionSource("nope.rs")}\``,
    ].join("\n");
    expect(extractMarkers(markdown)).toEqual(["seq"]);
    expect(extractFileRefs(markdown)).toEqual(["src/a.rs"]);
    expect(splitBodyRefs(`see ${file}`)).toEqual([
      { type: "text", value: "see " },
      { type: "file", path: "src/a.rs", label: "a.rs" },
    ]);
    expect(knowledgePreview(file, 1)).toBe("a.rs");
  });

  it("reads a symbol citation in order and skips code", () => {
    const symbol = symbolMentionSource("src/session/manager.rs", {
      symbol: "impl SessionManager › fn append_reminder",
      lines: "2148-2165",
      label: "fn append_reminder",
    });
    const range = symbolMentionSource("src/a.rs", { lines: "4-9", label: "a.rs" });
    const markdown = [
      `see ${symbol} and ${mentionSource("seq")}`,
      "```",
      symbolMentionSource("skip.rs", { symbol: "fn hidden", label: "fn hidden" }),
      "```",
      range,
    ].join("\n");
    expect(extractFileRefs(markdown)).toEqual(["src/session/manager.rs", "src/a.rs"]);
    expect(extractMarkers(markdown)).toEqual(["seq"]);
    expect(splitBodyRefs(`see ${symbol} then ${range}`)).toEqual([
      { type: "text", value: "see " },
      {
        type: "symbol",
        path: "src/session/manager.rs",
        symbol: "impl SessionManager › fn append_reminder",
        lines: "2148-2165",
        label: "fn append_reminder",
      },
      { type: "text", value: " then " },
      {
        type: "symbol",
        path: "src/a.rs",
        symbol: null,
        lines: "4-9",
        label: "a.rs",
      },
    ]);
    expect(knowledgePreview(symbol, 1)).toBe("fn append_reminder");
  });

  it("rejects a parent segment and an absolute path", () => {
    expect(isWorkspaceFileRef("src/a.rs")).toBe(true);
    expect(isWorkspaceFileRef("src")).toBe(true);
    expect(isWorkspaceFileRef("../secret")).toBe(false);
    expect(isWorkspaceFileRef("/etc/passwd")).toBe(false);
    expect(isWorkspaceFileRef("C:/abs")).toBe(false);
    expect(isWorkspaceFileRef("src/../a.rs")).toBe(false);
  });
});
