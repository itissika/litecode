import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useEditorStore } from "../../stores/editorStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { MentionText } from "./MentionText";

afterEach(() => {
  cleanup();
});

describe("MentionText", () => {
  it("draws node, file, and symbol capsules and leaves the prose to markdown", async () => {
    const seqNode = { id: "seq", key: "seq", path: "arch/seq.md", status: "enabled" };
    useKnowledgeStore.setState({
      byKey: new Map([["seq", seqNode]]),
      byId: new Map([["seq", seqNode]]),
      byLogicalPath: new Map([["knowledge/arch/seq.md", seqNode]]),
      visibility: "public",
    } as never);
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt });
    const text =
      "see [seq](knowledge/arch/seq.md) and [a.rs](src/a.rs) then [Store.save](src/a.rs#Store.save)";
    render(<MentionText text={text} />);
    fireEvent.click(await screen.findByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    fireEvent.click(screen.getByRole("button", { name: "a.rs : save" }));
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    expect(openFileAt).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "a.rs" }).className).toContain(
      "knowledge-token-label",
    );
    expect(screen.getByText(/see/)).toBeTruthy();
    const after = screen.getByText(/then/);
    expect(after.closest(".agent-markdown")?.classList.contains("is-inline")).toBe(true);
  });

  it("keeps ordinary text on the capsule's line and starts a new paragraph after a blank line", () => {
    render(<MentionText text={"[a.rs](src/a.rs) stays here\n\nnext paragraph"} />);
    const inline = document.querySelector(".agent-markdown.is-inline");
    expect(inline?.textContent).toContain("stays here");
    const blocks = [...document.querySelectorAll(".agent-markdown")].filter(
      (node) => !node.classList.contains("is-inline"),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.textContent).toContain("next paragraph");
  });
});
