import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { fileMentionSource, mentionSource, symbolMentionSource } from "../../lib/knowledge/markers";
import { useEditorStore } from "../../stores/editorStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { MentionText } from "./MentionText";

afterEach(() => {
  cleanup();
});

describe("MentionText", () => {
  it("draws node, file, and symbol capsules and leaves the prose to markdown", async () => {
    useKnowledgeStore.setState({
      byKey: new Map([["seq", { id: "seq", key: "seq", status: "active" }]]),
      byId: new Map([["seq", { id: "seq", key: "seq", status: "active" }]]),
    } as never);
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFileAt });
    const text = `see ${mentionSource("seq")} and ${fileMentionSource("src/a.rs")} then ${symbolMentionSource("src/a.rs", { symbol: "fn save", lines: "4-9", label: "fn save" })}`;
    render(<MentionText text={text} />);
    fireEvent.click(await screen.findByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    fireEvent.click(screen.getByRole("button", { name: "a.rs : fn save" }));
    expect(openFileAt).toHaveBeenCalledWith("src/a.rs", 4);
    expect(screen.getByRole("button", { name: ".../src/a.rs" }).className).toContain(
      "knowledge-token-label",
    );
    expect(screen.getByText(/see/)).toBeTruthy();
    const after = screen.getByText(/then/);
    expect(after.closest(".agent-markdown")?.classList.contains("is-inline")).toBe(true);
  });

  it("keeps ordinary text on the capsule's line and starts a new paragraph after a blank line", () => {
    const file = fileMentionSource("src/a.rs");
    render(<MentionText text={`${file} stays here\n\nnext paragraph`} />);
    const inline = document.querySelector(".agent-markdown.is-inline");
    expect(inline?.textContent).toContain("stays here");
    const blocks = [...document.querySelectorAll(".agent-markdown")].filter(
      (node) => !node.classList.contains("is-inline"),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.textContent).toContain("next paragraph");
  });
});
