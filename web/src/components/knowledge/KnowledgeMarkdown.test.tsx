import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeMarkdown } from "./KnowledgeMarkdown";

afterEach(() => {
  cleanup();
  useKnowledgeStore.setState({
    focusedId: null,
    focusNonce: 0,
    flashId: null,
    flashNonce: 0,
    expanded: new Set(),
  });
});

describe("KnowledgeMarkdown", () => {
  it("jumps to a registered citation", () => {
    render(<KnowledgeMarkdown sourceId={1} text="see [[seq]]" />);
    fireEvent.click(screen.getByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe(2);
    expect(useKnowledgeStore.getState().expanded.has(2)).toBe(false);
  });

  it("marks an unknown key and does not focus", () => {
    render(<KnowledgeMarkdown sourceId={14} text="[[not-a-node]]" />);
    const chip = screen.getByRole("button", { name: "not-a-node" });
    expect(chip.getAttribute("aria-invalid")).toBe("true");
    expect(chip.hasAttribute("disabled")).toBe(true);
    fireEvent.click(chip);
    expect(useKnowledgeStore.getState().focusedId).toBeNull();
  });

  it("does not turn a marker inside a code block into a citation", () => {
    render(<KnowledgeMarkdown sourceId={1} text={"```\n[[seq]]\n```"} />);
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });
});
