import { describe, expect, it } from "vitest";

import { splitMarkdownBlocks } from "./markdownBlocks";

function expectSplit(text: string, blocks: string[]) {
  const actual = splitMarkdownBlocks(text);
  expect(actual).toEqual(blocks);
  expect(actual.join("")).toBe(text);
}

describe("splitMarkdownBlocks", () => {
  it("keeps an empty document as one block", () => {
    expect(splitMarkdownBlocks("")).toEqual([""]);
  });

  it("keeps a paragraph with no blank line as one block", () => {
    const text = `word ${"z".repeat(8_000)}`;
    expectSplit(text, [text]);
  });

  it("cuts at a blank line between paragraphs", () => {
    expectSplit("one\n\ntwo", ["one\n\n", "two"]);
  });

  it("cuts a short document the same way as a long one", () => {
    expectSplit("one\n\ntwo\n\nthree", ["one\n\n", "two\n\n", "three"]);
  });

  it("does not cut a blank line inside an open fence", () => {
    const text = "```\ncode\n\nstill\n```";
    expectSplit(text, [text]);
  });

  it("cuts after a closed fence", () => {
    expectSplit("```\ncode\n\nstill\n```\n\nafter", [
      "```\ncode\n\nstill\n```\n\n",
      "after",
    ]);
  });

  it("treats a tilde fence like a backtick fence", () => {
    const text = "~~~\ncode\n\nstill\n~~~";
    expectSplit(text, [text]);
  });

  it("keeps a loose list together", () => {
    expectSplit("- a\n\n- b\n\nnext", ["- a\n\n- b\n\n", "next"]);
  });

  it("keeps a list continuation that arrives after the blank line", () => {
    expectSplit("- a\n\n", ["- a\n\n"]);
    expectSplit("- a\n\n  more", ["- a\n\n  more"]);
  });

  it("keeps a block quote together across a blank line", () => {
    expectSplit("> one\n>\n> two\n\nafter", ["> one\n>\n> two\n\n", "after"]);
  });

  it("keeps a table together", () => {
    const table = "| a | b |\n| --- | --- |\n| 1 | 2 |";
    expectSplit(`${table}\n\nafter`, [`${table}\n\n`, "after"]);
  });

  it("keeps a setext heading with its text", () => {
    expectSplit("Title\n---\n\nbody", ["Title\n---\n\n", "body"]);
  });

  it("splits a heading from the following paragraph", () => {
    expectSplit("# Title\n\nbody", ["# Title\n\n", "body"]);
  });

  it("keeps a link definition in the same block as its reference", () => {
    const text = "see [a][1]\n\n[1]: https://example.com";
    expectSplit(text, [text]);
  });

  it("keeps a footnote reference and definition together", () => {
    const text = "note[^1]\n\n[^1]: detail";
    expectSplit(text, [text]);
  });

  it("keeps earlier blocks when text is appended", () => {
    const base = "one\n\ntail";
    const more = `${base} grows`;
    const first = splitMarkdownBlocks(base);
    const second = splitMarkdownBlocks(more);
    expect(second[0]).toBe(first[0]);
    expect(second[0]).toBe("one\n\n");
    expect(second[1]?.startsWith("tail")).toBe(true);
    expect(second.join("")).toBe(more);
  });

  it("keeps a closed fence when the following paragraph grows", () => {
    const base = "```\ncode\n```\n\ntail";
    const first = splitMarkdownBlocks(base);
    const second = splitMarkdownBlocks(`${base} grows`);
    expect(second[0]).toBe(first[0]);
    expect(second[0]).toBe("```\ncode\n```\n\n");
    expect(second.map((block) => block).join("")).toBe(`${base} grows`);
  });
});
