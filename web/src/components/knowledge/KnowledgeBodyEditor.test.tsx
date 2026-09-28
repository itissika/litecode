import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../lib/knowledge/fixture";
import { mentionSource } from "../../lib/knowledge/markers";
import { knowledgeSnapshot, useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeBodyEditor } from "./KnowledgeBodyEditor";

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

describe("KnowledgeBodyEditor mention chip", () => {
  it("jumps to the cited card and keeps the shortcode", async () => {
    const onChange = vi.fn();
    const source = `见 ${mentionSource("seq")}`;
    render(
      <KnowledgeBodyEditor
        label="正文"
        sourceId="session"
        value={source}
        candidates={["seq"]}
        rows={3}
        onChange={onChange}
      />,
    );
    const chip = await screen.findByRole("button", { name: "seq" });
    fireEvent.click(chip);
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "seq" })).toBeTruthy();
  });

  it("does not jump when the id is missing", async () => {
    render(
      <KnowledgeBodyEditor
        label="正文"
        sourceId="broken-marker"
        value={mentionSource("not-a-node")}
        candidates={[]}
        rows={3}
        onChange={() => undefined}
      />,
    );
    expect(await screen.findByText("not-a-node")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "not-a-node" })).toBeNull();
    expect(useKnowledgeStore.getState().focusedId).toBeNull();
  });
});
