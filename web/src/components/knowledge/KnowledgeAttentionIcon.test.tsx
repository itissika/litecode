import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { KnowledgeAttentionIcon } from "./KnowledgeAttentionIcon";

const drift: KnowledgeIssue = {
  nodeId: "a",
  severity: "warning",
  code: "symbol_drift",
  message: 'Symbol "run" in "src/a.rs" differs from HEAD.',
};

afterEach(() => {
  cleanup();
});

describe("KnowledgeAttentionIcon", () => {
  it("renders nothing without warnings", () => {
    const { container } = render(<KnowledgeAttentionIcon warnings={[]} />);
    expect(container.querySelector(".knowledge-attention-icon")).toBeNull();
  });

  it("marks the warnings with their messages as the title", () => {
    render(<KnowledgeAttentionIcon warnings={[drift]} />);
    const mark = screen.getByRole("img", { name: "1 个问题需要关注" });
    expect(mark.getAttribute("title")).toBe(drift.message);
  });
});
