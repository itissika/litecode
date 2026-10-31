import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { KnowledgeAttentionIcon } from "./KnowledgeAttentionIcon";

const mismatch: KnowledgeIssue = {
  nodeId: "a",
  severity: "warning",
  code: "filename_mismatch",
  message: 'Filename "b.md" does not match declaration "a".',
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
    render(<KnowledgeAttentionIcon warnings={[mismatch]} />);
    const mark = screen.getByRole("img", { name: "1 issues need attention" });
    expect(mark.getAttribute("title")).toBe(mismatch.message);
  });
});
