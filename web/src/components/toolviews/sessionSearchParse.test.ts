import { describe, expect, it } from "vitest";

import { parseSessionSearch } from "./sessionSearchParse";

const DOT = " \u00b7 ";
const DASH = " \u2014 ";

function view(lines: string[], trailingNewline = true): string {
  const text = lines.join("\n");
  return trailingNewline ? `${text}\n` : text;
}

describe("parseSessionSearch", () => {
  it("parses one group with a line range", () => {
    const parsed = parseSessionSearch(
      view([
        `### ABCD1234${DOT}just now${DOT}1 Matches`,
        "L2-4: user",
        "  one",
        "  VIEW_NEEDLE two",
        "  three",
      ]),
    );
    expect(parsed).toEqual({
      kind: "hits",
      groups: [
        {
          handle: "ABCD1234",
          age: "just now",
          count: 1,
          hits: [
            {
              from: 2,
              to: 4,
              label: "user",
              lines: ["one", "VIEW_NEEDLE two", "three"],
            },
          ],
        },
      ],
    });
  });

  it("parses a single line and a label that contains a middle dot", () => {
    const parsed = parseSessionSearch(
      view([
        `### Y4WZFXGZ${DOT}1w ago${DOT}13 Matches`,
        "L16037: tool call \u00b7 bash",
        '  bash({"command": "ls"})',
      ]),
    );
    expect(parsed).toMatchObject({
      kind: "hits",
      groups: [
        {
          count: 13,
          hits: [
            {
              from: 16037,
              label: "tool call \u00b7 bash",
              lines: ['bash({"command": "ls"})'],
            },
          ],
        },
      ],
    });
  });

  it("keeps each header count when the card carries fewer hits", () => {
    const parsed = parseSessionSearch(
      view([
        `### AAAA1111${DOT}1w ago${DOT}10 Matches`,
        "L3: user",
        "  only one line of this session",
        `### BBBB2222${DOT}2d ago${DOT}3 Matches`,
        "L9: assistant message",
        "  elsewhere",
      ]),
    );
    expect(parsed).toMatchObject({
      kind: "hits",
      groups: [
        { handle: "AAAA1111", age: "1w ago", count: 10 },
        { handle: "BBBB2222", age: "2d ago", count: 3 },
      ],
    });
    if (parsed?.kind !== "hits") throw new Error("expected hits");
    expect(parsed.groups[0].hits).toHaveLength(1);
    expect(parsed.groups[1].hits).toHaveLength(1);
  });

  it("parses a spill footer and the more-in line", () => {
    const parsed = parseSessionSearch(
      view([
        `### Y4WZFXGZ${DOT}1w ago${DOT}13 Matches`,
        "L16037: tool call \u00b7 bash",
        '  bash({"command": "ls"})',
        "",
        `Showing 1 of 13 hits; the remaining 12 are in .litecode/bash/session_search_bf45af87.txt${DASH}read or grep it.`,
        "More in: 5NFV5Z09 17, FY53N8KB 4.",
      ]),
    );
    expect(parsed).toMatchObject({
      kind: "hits",
      footer: {
        shown: 1,
        total: 13,
        remaining: 12,
        location: ".litecode/bash/session_search_bf45af87.txt",
        more: "5NFV5Z09 17, FY53N8KB 4",
      },
    });
  });

  it("parses an empty result, including a trailing newline", () => {
    const text = "No matching session transcript context for query '残影'.";
    expect(parseSessionSearch(`${text}\n`)).toEqual({
      kind: "empty",
      text,
    });
  });

  it("returns null for an error string", () => {
    expect(
      parseSessionSearch("Error: session ref 'x' matched no sessions"),
    ).toBeNull();
  });

  it("accepts CRLF and a missing trailing newline", () => {
    const lines = [
      `### ABCD1234${DOT}just now${DOT}1 Matches`,
      "L8: user",
      "  short",
    ];
    const lf = parseSessionSearch(view(lines, false));
    const crlf = parseSessionSearch(view(lines).replace(/\n/g, "\r\n"));
    expect(lf).toEqual(crlf);
    expect(lf).toMatchObject({ kind: "hits" });
  });

  it("treats indented structural-looking lines as body", () => {
    const parsed = parseSessionSearch(
      view([
        `### HHHH${DOT}just now${DOT}1 Matches`,
        "L2: user",
        "  ### not a header",
        "  L1: x",
      ]),
    );
    expect(parsed).toMatchObject({
      kind: "hits",
      groups: [
        {
          hits: [{ lines: ["### not a header", "L1: x"] }],
        },
      ],
    });
  });

  it("returns null for an unrecognized line", () => {
    expect(
      parseSessionSearch(
        view([
          `### HHHH${DOT}just now${DOT}1 Matches`,
          "L2: user",
          "  ok",
          "NOT A LINE",
        ]),
      ),
    ).toBeNull();
  });

  it("returns null when a hit has no body", () => {
    expect(
      parseSessionSearch(
        view([`### HHHH${DOT}just now${DOT}1 Matches`, "L2: user"]),
      ),
    ).toBeNull();
  });

  it("returns null for the old headers-first shape", () => {
    expect(
      parseSessionSearch(
        view([
          `### AAAA${DOT}just now${DOT}1 Matches`,
          `### BBBB${DOT}1d ago${DOT}1 Matches`,
          "L2: user",
          "  one",
        ]),
      ),
    ).toBeNull();
  });

  it("returns null when a blank line is not the footer", () => {
    expect(
      parseSessionSearch(
        view([
          `### HHHH${DOT}just now${DOT}1 Matches`,
          "L2: user",
          "  one",
          "",
          "  two",
        ]),
      ),
    ).toBeNull();
  });
});
