import { describe, expect, it } from "vitest";

import {
  MARKDOWN_BLOCK_MAX_CHARS,
  MARKDOWN_BLOCK_MIN_CHARS,
  estimateBlockHeight,
  splitMarkdownBlocks,
} from "./markdownBlocks";

describe("splitMarkdownBlocks", () => {
  it("keeps a short run as one block", () => {
    const text = "one\n\ntwo\n\nthree";
    const blocks = splitMarkdownBlocks(text);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.text).toBe(text);
  });

  it("cuts at a blank line once the block is long enough", () => {
    const text = `${"a".repeat(MARKDOWN_BLOCK_MIN_CHARS)}\n\n${"b".repeat(40)}`;
    const blocks = splitMarkdownBlocks(text);
    expect(blocks.map((block) => block.text)).toEqual([
      `${"a".repeat(MARKDOWN_BLOCK_MIN_CHARS)}\n\n`,
      "b".repeat(40),
    ]);
  });

  it("merges paragraphs that are still under the minimum", () => {
    const text = `${"a".repeat(400)}\n\n${"b".repeat(300)}\n\nend`;
    expect(splitMarkdownBlocks(text)).toHaveLength(1);
  });

  it("does not cut a blank line inside an open fence", () => {
    const text = `\`\`\`\n${"x".repeat(MARKDOWN_BLOCK_MIN_CHARS)}\n\ny\n\`\`\``;
    const blocks = splitMarkdownBlocks(text);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.text).toBe(text);
  });

  it("cuts after a closed fence when the block is long enough", () => {
    const text = `\`\`\`\n${"x".repeat(MARKDOWN_BLOCK_MIN_CHARS)}\n\`\`\`\n\nafter`;
    const blocks = splitMarkdownBlocks(text);
    expect(blocks).toHaveLength(2);
    expect(blocks[0]?.text.startsWith("```")).toBe(true);
    expect(blocks[0]?.text.endsWith("```\n\n")).toBe(true);
    expect(blocks[1]?.text).toBe("after");
  });

  it("treats a tilde fence like a backtick fence", () => {
    const text = `~~~\n${"x".repeat(MARKDOWN_BLOCK_MIN_CHARS)}\n\ny\n~~~`;
    expect(splitMarkdownBlocks(text)).toHaveLength(1);
  });

  it("force-cuts a long run at the previous newline", () => {
    const unit = `${"y".repeat(80)}\n`;
    const text = unit.repeat(
      Math.ceil((MARKDOWN_BLOCK_MAX_CHARS + 100) / unit.length),
    );
    const blocks = splitMarkdownBlocks(text);
    expect(blocks.length).toBeGreaterThan(1);
    expect(blocks.slice(0, -1).every((block) => block.text.endsWith("\n"))).toBe(
      true,
    );
    expect(
      blocks.slice(0, -1).every((block) => block.text.length <= MARKDOWN_BLOCK_MAX_CHARS),
    ).toBe(true);
    expect(blocks.map((block) => block.text).join("")).toBe(text);
  });

  it("force-cuts a run with no newline on a character boundary", () => {
    const text = "z".repeat(MARKDOWN_BLOCK_MAX_CHARS + 10);
    const blocks = splitMarkdownBlocks(text);
    expect(blocks[0]?.text).toHaveLength(MARKDOWN_BLOCK_MAX_CHARS);
    expect(blocks.map((block) => block.text).join("")).toBe(text);
  });

  it("keeps earlier blocks when text is appended", () => {
    const base = `${"a".repeat(MARKDOWN_BLOCK_MIN_CHARS)}\n\ntail`;
    const more = `${base} grows`;
    const first = splitMarkdownBlocks(base);
    const second = splitMarkdownBlocks(more);
    expect(first.length).toBeGreaterThan(1);
    expect(second[0]).toEqual(first[0]);
    expect(second).toHaveLength(first.length);
    expect(second.at(-1)?.text.startsWith(first.at(-1)?.text ?? "")).toBe(true);
  });

  it("keeps closed blocks when a long tail grows", () => {
    const unit = `${"y".repeat(80)}\n`;
    const base = unit.repeat(
      Math.ceil((MARKDOWN_BLOCK_MAX_CHARS + 400) / unit.length),
    );
    const first = splitMarkdownBlocks(base);
    const second = splitMarkdownBlocks(`${base}END`);
    expect(first.length).toBeGreaterThan(1);
    for (let i = 0; i < first.length - 1; i += 1) {
      expect(second[i]).toEqual(first[i]);
    }
    expect(second.map((block) => block.text).join("")).toBe(`${base}END`);
  });
});

describe("estimateBlockHeight", () => {
  it("counts wrapped lines at the transcript measure", () => {
    expect(estimateBlockHeight("")).toBe(25);
    expect(estimateBlockHeight("a".repeat(72))).toBe(25);
    expect(estimateBlockHeight("a".repeat(73))).toBe(50);
    expect(estimateBlockHeight("a\n")).toBe(50);
  });
});
