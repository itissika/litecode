import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { bindDockview } from "../../dockview/workbench/host";

import {
  knowledgeFixture,
  knowledgeFolderFixture,
} from "../../test/fixtures/knowledge";
import type { KnowledgeIssue } from "../../lib/knowledge/types";
import {
  knowledgeSnapshot,
  useKnowledgeStore,
} from "../../stores/knowledgeStore";
import { KnowledgeBrowser } from "./KnowledgeBrowser";

beforeEach(() => {
  useKnowledgeStore.setState({
    ...knowledgeSnapshot(knowledgeFixture, knowledgeFolderFixture),
    unknown: [],
    expanded: new Set(),
    graphExpanded: new Set(),
    focusedId: null,
    flashId: null,
    flashNonce: 0,
    loading: false,
    error: null,
    load: async () => {},
  });
});

afterEach(() => {
  bindDockview(null);
  cleanup();
});

const drift: KnowledgeIssue = {
  nodeId: "session",
  severity: "warning",
  code: "symbol_drift",
  message: 'Symbol "run" in "src/a.rs" differs from HEAD.',
};

const inactive: KnowledgeIssue = {
  nodeId: "seq",
  severity: "warning",
  code: "inactive_target",
  message: 'Cites disabled "temperature".',
};

describe("KnowledgeBrowser attention mark", () => {
  it("puts the amber exclamation after a warning node title only", () => {
    useKnowledgeStore.setState({
      issuesByNode: new Map([
        ["session", [drift]],
        ["seq", [inactive]],
      ]),
    });
    const { container } = render(<KnowledgeBrowser />);
    expect(
      container.querySelector(
        '[data-knowledge-id="session"] .knowledge-attention-icon',
      ),
    ).not.toBeNull();
    expect(
      container.querySelector(
        '[data-knowledge-id="seq"] .knowledge-attention-icon',
      ),
    ).toBeNull();
  });

  it("uses the file-tree row scale for node titles", () => {
    render(<KnowledgeBrowser />);
    const title = screen.getByText("session");
    const row = title.closest(".knowledge-card");
    expect(row?.className).toContain("text-sm");
    expect(row?.className).toContain("px-2");
    expect(title.className).not.toContain("font-code");
  });

  it("lists node keys without a summary", () => {
    useKnowledgeStore.setState({ expanded: new Set(["session"]) });
    const { container } = render(<KnowledgeBrowser />);
    expect(container.querySelector(".knowledge-side-summary")).toBeNull();
    expect(container.textContent).toContain("session");
    expect(container.textContent).not.toContain("单调递增");
  });

  it("opens the graph from a node and has no Open Graph button", () => {
    const addPanel = vi.fn();
    bindDockview({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(() => ({ id: "fresh" })),
      groups: [],
    } as never);
    render(<KnowledgeBrowser />);
    expect(screen.queryByRole("button", { name: "Open Graph" })).toBeNull();
    fireEvent.click(screen.getByText("session"));
    expect(useKnowledgeStore.getState().focusedId).toBe("session");
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "knowledge-graph",
        component: "knowledgeGraph",
      }),
    );
  });
});
