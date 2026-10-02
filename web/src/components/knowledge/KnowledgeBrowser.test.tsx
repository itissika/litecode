import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  knowledgeFixture,
  knowledgeFolderFixture,
} from "../../lib/knowledge/fixture";
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
});
