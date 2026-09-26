import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { FunctionCallOutputItem } from "../../api/types";
import { SessionSearchToolView } from "./SessionSearchToolView";

afterEach(() => {
  cleanup();
});

const DOT = " \u00b7 ";
const DASH = " \u2014 ";

function output(text: string): FunctionCallOutputItem {
  return { type: "function_call_output", call_id: "call_1", output: text };
}

const TWO_HITS = [
  `### ABCD1234${DOT}2d ago${DOT}2 Matches`,
  "L2-4: user",
  "  hello   world",
  "  next",
  "L9: assistant message",
  "  gamma",
  "  delta",
].join("\n");

describe("SessionSearchToolView", () => {
  it("renders one row per hit and collapses whitespace in the preview", () => {
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "NEEDLE_QUERY" }}
        output={output(TWO_HITS)}
      />,
    );
    expect(screen.getAllByTestId("session-search-hit")).toHaveLength(2);
    expect(screen.getAllByTestId("session-search-preview")[0].textContent).toBe(
      "hello world next",
    );
    expect(screen.getByText("L2-4")).toBeTruthy();
    expect(screen.getByText("user")).toBeTruthy();
    expect(screen.getByTestId("session-search-count").textContent).toBe(
      "2 matches",
    );
    expect(screen.queryByText("NEEDLE_QUERY")).toBeNull();
    expect(screen.queryByRole("button", { name: /NEEDLE_QUERY/ })).toBeNull();
  });

  it("expands and collapses the full body in place", () => {
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "q" }}
        output={output(TWO_HITS)}
      />,
    );
    expect(screen.queryByTestId("session-search-hit-body")).toBeNull();
    const [first, second] = screen.getAllByRole("button");
    fireEvent.click(first);
    expect(screen.getByTestId("session-search-hit-body").textContent).toBe(
      "hello   world\nnext",
    );
    expect(screen.getAllByTestId("session-search-hit-body")).toHaveLength(1);
    fireEvent.click(second);
    const bodies = screen.getAllByTestId("session-search-hit-body");
    expect(bodies.map((node) => node.textContent)).toEqual([
      "hello   world\nnext",
      "gamma\ndelta",
    ]);
    fireEvent.click(first);
    expect(screen.getByTestId("session-search-hit-body").textContent).toBe(
      "gamma\ndelta",
    );
  });

  it("shows the header total and the spill footer", () => {
    const text = [
      `### Y4WZFXGZ${DOT}1w ago${DOT}13 Matches`,
      "L16037: tool call \u00b7 bash",
      '  bash({"command": "ls"})',
      "",
      `Showing 1 of 13 hits; the remaining 12 are in .litecode/bash/session_search_bf45af87.txt${DASH}read or grep it.`,
      "More in: 5NFV5Z09 17, FY53N8KB 4.",
    ].join("\n");
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "ls", session_id: "Y4WZFXGZ" }}
        output={output(text)}
      />,
    );
    expect(screen.getAllByTestId("session-search-hit")).toHaveLength(1);
    expect(screen.getByTestId("session-search-count").textContent).toBe(
      "13 matches",
    );
    expect(screen.getByTestId("session-search-scope").textContent).toContain(
      "session Y4WZFXGZ",
    );
    const footer = screen.getByTestId("session-search-footer");
    expect(footer.textContent).toContain("Showing 1 of 13");
    expect(footer.textContent).toContain(
      ".litecode/bash/session_search_bf45af87.txt",
    );
    expect(footer.textContent).toContain("More in: 5NFV5Z09 17, FY53N8KB 4");
  });

  it("uses a singular match label and skips the caret when the body is one short line", () => {
    const text = [
      `### ABCD1234${DOT}just now${DOT}1 Matches`,
      "L8: user",
      "  short",
    ].join("\n");
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "short" }}
        output={output(text)}
      />,
    );
    expect(screen.getByTestId("session-search-count").textContent).toBe(
      "1 match",
    );
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("tucks non-primary fields into the info icon", () => {
    const text = [
      `### ABCD1234${DOT}just now${DOT}1 Matches`,
      "L8: user",
      "  short",
    ].join("\n");
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "q", session_id: "SID", limit: 4 }}
        output={output(text)}
      />,
    );
    expect(screen.getByText("limit")).toBeTruthy();
    expect(screen.getByText("4")).toBeTruthy();
    expect(screen.queryByText("q")).toBeNull();
  });

  it("shows a failed result in red and does not parse it", () => {
    render(
      <SessionSearchToolView
        name="session_search"
        status="failed"
        input={{ query: "q" }}
        output={output(TWO_HITS)}
      />,
    );
    expect(screen.queryByTestId("session-search-hit")).toBeNull();
    const raw = screen.getByTestId("session-search-raw");
    expect(raw.className).toContain("text-(--_dk-red-500)");
    expect(raw.textContent).toContain("### ABCD1234");
  });

  it("falls back to the raw text when the grammar does not match", () => {
    const text = "nope\n### still raw";
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "q" }}
        output={output(text)}
      />,
    );
    expect(screen.queryByTestId("session-search-hit")).toBeNull();
    const raw = screen.getByTestId("session-search-raw");
    expect(raw.textContent).toBe(text);
    expect(raw.className).toContain("text-(--_dk-text-secondary)");
    expect(raw.className).not.toContain("text-(--_dk-red-500)");
  });

  it("shows an empty result in muted text", () => {
    const text = "No matching session transcript context for query '残影'.";
    render(
      <SessionSearchToolView
        name="session_search"
        status="ok"
        input={{ query: "残影" }}
        output={output(text)}
      />,
    );
    const empty = screen.getByTestId("session-search-empty");
    expect(empty.textContent).toBe(text);
    expect(empty.className).toContain("text-(--_dk-text-muted)");
    expect(screen.queryByTestId("session-search-hit")).toBeNull();
  });
});
