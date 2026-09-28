import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../lib/knowledge/fixture";
import { knowledgeSnapshot, useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeMarkdown } from "./KnowledgeMarkdown";

beforeEach(() => {
  useKnowledgeStore.setState({
    ...knowledgeSnapshot(knowledgeFixture, knowledgeFolderFixture),
    expanded: new Set(),
    graphExpanded: new Set(),
    focusedId: null,
    focusNonce: 0,
    flashId: null,
    flashNonce: 0,
    loading: false,
    error: null,
  });
});

afterEach(() => {
  cleanup();
});

describe("KnowledgeMarkdown", () => {
  it("jumps to a registered citation", () => {
    render(<KnowledgeMarkdown sourceId="session" text="see [[node : seq]]" />);
    fireEvent.click(screen.getByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(useKnowledgeStore.getState().expanded.has("seq")).toBe(false);
  });

  it("marks an unknown key and does not focus", () => {
    render(
      <KnowledgeMarkdown sourceId="broken-marker" text="[[node : not-a-node]]" />,
    );
    const chip = screen.getByRole("button", { name: "not-a-node" });
    expect(chip.getAttribute("aria-invalid")).toBe("true");
    expect(chip.hasAttribute("disabled")).toBe(true);
    fireEvent.click(chip);
    expect(useKnowledgeStore.getState().focusedId).toBeNull();
  });

  it("does not turn a marker inside a code block into a citation", () => {
    render(
      <KnowledgeMarkdown sourceId="session" text={"```\n[[node : seq]]\n```"} />,
    );
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });

  it("does not treat a bare double-bracket as a citation", () => {
    render(<KnowledgeMarkdown sourceId="session" text="see [[seq]]" />);
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });
});
