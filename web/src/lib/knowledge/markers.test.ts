import { describe, expect, it } from "vitest";

import {
  citationMarkdown,
  extractNodeRelations,
  extractRefs,
  humanFileLabel,
  humanRangeLabel,
  humanSymbolLabel,
  isValidNodeIdentity,
  isWorkspaceFileRef,
  knowledgePreview,
  knowledgeRefKey,
  leafSymbol,
  parseLineAnchor,
  replaceCitationPath,
  replaceCitationPathPrefix,
  splitBodyRefs,
} from "./markers";

describe("human chip labels", () => {
  it("keeps the file name and leaves the path to the title", () => {
    expect(humanFileLabel("a.rs")).toBe("a.rs");
    expect(humanFileLabel("src/a.rs")).toBe("a.rs");
    expect(humanFileLabel("src/session/manager.rs")).toBe("manager.rs");
  });

  it("shows a symbol as the file name and the leaf segment", () => {
    expect(humanSymbolLabel("src/session/manager.rs", "Store.save")).toBe(
      "manager.rs : save",
    );
    expect(humanSymbolLabel("src/session/manager.rs", "Store::save")).toBe(
      "manager.rs : save",
    );
    expect(leafSymbol("Store.save")).toBe("save");
    expect(leafSymbol("save")).toBe("save");
  });

  it("shows a line range as file : span", () => {
    expect(humanRangeLabel("src/a.rs", "4-9")).toBe("a.rs : 4-9");
    expect(humanRangeLabel("src/a.rs", "4-4")).toBe("a.rs : 4");
    expect(humanRangeLabel("src/a.rs", "0-1")).toBe("a.rs");
  });
});

describe("standard Markdown link citations", () => {
  it("formats canonical links including angle-bracket paths with spaces", () => {
    expect(citationMarkdown({ path: "src/a.rs" })).toBe("[a.rs](src/a.rs)");
    expect(citationMarkdown({ path: "src/a.rs", lines: "4-9" })).toBe(
      "[a.rs:4-9](src/a.rs#L4-L9)",
    );
    expect(citationMarkdown({ path: "src/a.rs", symbol: "Store::save" })).toBe(
      "[Store.save](src/a.rs#Store.save)",
    );
    expect(citationMarkdown({ path: "docs/my note.md" })).toBe(
      "[my note.md](<docs/my note.md>)",
    );
  });

  it("splits workspace links and skips web URLs, images, and inline code", () => {
    const line =
      "see [seq](knowledge/seq.md), [web](https://example.com), ![img](a.png), `[c](src/skip.rs)`, and [save](src/a.rs#Store.save)";
    expect(splitBodyRefs(line)).toEqual([
      { type: "text", value: "see " },
      {
        type: "cite",
        label: "seq",
        path: "knowledge/seq.md",
        symbol: null,
        lines: null,
        badLines: null,
        raw: "[seq](knowledge/seq.md)",
      },
      {
        type: "text",
        value: ", [web](https://example.com), ![img](a.png), `[c](src/skip.rs)`, and ",
      },
      {
        type: "cite",
        label: "save",
        path: "src/a.rs",
        symbol: "Store.save",
        lines: null,
        badLines: null,
        raw: "[save](src/a.rs#Store.save)",
      },
    ]);
  });

  it("extracts refs in order, skips code fences, and flags bare #12 as bad_lines", () => {
    const markdown = [
      "see [a](src/a.rs#L4-9) and [b](src/b.rs#12-30)",
      "```rs",
      "[skip](src/skip.rs)",
      "```",
      "~~~",
      "[skip2](src/skip2.rs)",
      "~~~",
      "end [Store.save](src/a.rs#Store::save)",
    ].join("\n");
    expect(extractRefs(markdown)).toEqual([
      { kind: "range", path: "src/a.rs", lines: "4-9", ref_key: "range:src/a.rs\x004-9" },
      {
        kind: "range",
        path: "src/b.rs",
        lines: null,
        bad_lines: "12-30",
        ref_key: "range:src/b.rs\x0012-30",
      },
      {
        kind: "symbol",
        path: "src/a.rs",
        symbol: "Store.save",
        ref_key: "sym:src/a.rs\x00Store.save",
      },
    ]);
    expect(parseLineAnchor("L12")).toEqual({ start: 12, end: 12 });
    expect(parseLineAnchor("L12-30")).toEqual({ start: 12, end: 30 });
    expect(parseLineAnchor("L12-L30")).toEqual({ start: 12, end: 30 });
    expect(parseLineAnchor("12-30")).toBeNull();
  });

  it("folds valid bare-path knowledge links into relations and skips self", () => {
    const valid = new Map([
      ["knowledge/arch/seq.md", { key: "seq", path: "arch/seq.md" }],
      ["knowledge/arch/self.md", { key: "self", path: "arch/self.md" }],
    ]);
    const body =
      "[seq](knowledge/arch/seq.md) [self](knowledge/arch/self.md) [seq2](knowledge/arch/seq.md#L1) [code](src/a.rs)";
    expect(
      extractNodeRelations(body, (p) => valid.get(p), "arch/self.md"),
    ).toEqual(["seq"]);
  });

  it("rewrites citation paths and folder prefixes outside code", () => {
    const body = [
      "see [seq](knowledge/arch/seq.md) and [custom](knowledge/arch/seq.md)",
      "```",
      "[seq](knowledge/arch/seq.md)",
      "```",
      "`[seq](knowledge/arch/seq.md)`",
    ].join("\n");
    expect(replaceCitationPath(body, "knowledge/arch/seq.md", "knowledge/core/order.md")).toBe(
      [
        "see [order](knowledge/core/order.md) and [custom](knowledge/core/order.md)",
        "```",
        "[seq](knowledge/arch/seq.md)",
        "```",
        "`[seq](knowledge/arch/seq.md)`",
      ].join("\n"),
    );
    expect(replaceCitationPathPrefix(body, "knowledge/arch", "knowledge/moved")).toBe(
      [
        "see [seq](knowledge/moved/seq.md) and [custom](knowledge/moved/seq.md)",
        "```",
        "[seq](knowledge/arch/seq.md)",
        "```",
        "`[seq](knowledge/arch/seq.md)`",
      ].join("\n"),
    );
  });

  it("checks node identity (filename stem === key)", () => {
    expect(isValidNodeIdentity("arch/seq.md", "seq")).toBe(true);
    expect(isValidNodeIdentity("arch/other.md", "seq")).toBe(false);
    expect(isValidNodeIdentity("arch/bad.md", "")).toBe(false);
  });

  it("rejects parent segments and absolute paths", () => {
    expect(isWorkspaceFileRef("src/a.rs")).toBe(true);
    expect(isWorkspaceFileRef("src")).toBe(true);
    expect(isWorkspaceFileRef("../secret")).toBe(false);
    expect(isWorkspaceFileRef("/etc/passwd")).toBe(false);
    expect(isWorkspaceFileRef("C:/abs")).toBe(false);
    expect(isWorkspaceFileRef("src/../a.rs")).toBe(false);
    expect(isWorkspaceFileRef("文".repeat(512))).toBe(true);
    expect(isWorkspaceFileRef("文".repeat(513))).toBe(false);
  });

  it("computes syntax-only ref_keys", () => {
    expect(knowledgeRefKey({ path: "src\\a.rs" })).toBe("path:src/a.rs");
    expect(knowledgeRefKey({ path: "src/a.rs", lines: "7-7" })).toBe("range:src/a.rs\x007");
    expect(knowledgeRefKey({ path: "src/a.rs", badLines: "12" })).toBe("range:src/a.rs\x0012");
    expect(knowledgeRefKey({ path: "src/a.rs", symbol: "Store::save" })).toBe(
      "sym:src/a.rs\x00Store.save",
    );
  });
});

describe("knowledgePreview", () => {
  it("drops fences, reduces workspace links to labels, and clips long inline code", () => {
    const value =
      "alpha [序号](knowledge/seq.md)\n\n```\n[hidden](knowledge/hidden.md)\n```\nbeta `node : seq`";
    expect(knowledgePreview(value, 2)).toBe("alpha 序号\nbeta `node : seq`");
    const long = "a".repeat(30);
    expect(knowledgePreview(`\`${long}\``, 1)).toBe(`\`${"a".repeat(24)}…\``);
  });
});
