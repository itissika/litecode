import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { FunctionCallOutputItem } from "../../api/types";
import { CodeSearchToolView } from "./CodeSearchToolView";

afterEach(() => {
  cleanup();
});

function output(text: string): FunctionCallOutputItem {
  return { type: "function_call_output", call_id: "call_1", output: text };
}

describe("CodeSearchToolView", () => {
  it("shows sync% at the top when the first line is present", () => {
    render(
      <CodeSearchToolView
        name="code_search"
        status="ok"
        input={{ query: "foo" }}
        output={output("sync_pct: 87% (syncing)\npath.ts:1: hit")}
      />,
    );
    expect(screen.getByTestId("tool-sync-pct-value").textContent).toBe("87%");
    expect(screen.getByTestId("tool-sync-pct-label").textContent).toBe(
      "syncing",
    );
    expect(screen.getByTestId("code-search-body").textContent).toBe(
      "path.ts:1: hit",
    );
    expect(screen.getByTestId("code-search-body").textContent).not.toContain(
      "sync_pct",
    );
  });

  it("omits the sync head when the field is absent", () => {
    render(
      <CodeSearchToolView
        name="code_search"
        status="ok"
        input={{ query: "foo" }}
        output={output("path.ts:1: hit")}
      />,
    );
    expect(screen.queryByTestId("tool-sync-pct")).toBeNull();
    expect(screen.getByTestId("code-search-body").textContent).toBe(
      "path.ts:1: hit",
    );
  });

  it("keeps sync head then empty-hit body (no fake 100%)", () => {
    render(
      <CodeSearchToolView
        name="code_search"
        status="ok"
        input={{ query: "zzz" }}
        output={output("sync_pct: 42%\nNo matching code for query 'zzz'.")}
      />,
    );
    expect(screen.getByTestId("tool-sync-pct-value").textContent).toBe("42%");
    expect(screen.getByTestId("code-search-body").textContent).toBe(
      "No matching code for query 'zzz'.",
    );
  });

  it("WP-138: renders v2 words and chunks, keeps unrecognised line in body", () => {
    const { unmount } = render(
      <CodeSearchToolView
        name="code_search"
        status="ok"
        input={{ query: "q" }}
        output={output("sync_pct: failed (model_missing)\nNo matching code.")}
      />,
    );
    expect(screen.getByTestId("tool-sync-pct-value").textContent).toBe("failed");
    unmount();
    const r2 = render(
      <CodeSearchToolView
        name="code_search"
        status="ok"
        input={{ query: "q" }}
        output={output("sync_pct: 98% (refreshing, embedding) chunks 9800/10000\npath.ts:1: hit")}
      />,
    );
    expect(screen.getByTestId("tool-sync-pct-chunks").textContent).toBe("9800/10000 chunks");
    r2.unmount();
    render(
      <CodeSearchToolView
        name="code_search"
        status="ok"
        input={{ query: "q" }}
        output={output("sync_pct: 87\npath.ts:1: hit")}
      />,
    );
    expect(screen.queryByTestId("tool-sync-pct")).toBeNull();
    expect(screen.getByText(/sync_pct: 87/)).toBeTruthy();
  });
});
