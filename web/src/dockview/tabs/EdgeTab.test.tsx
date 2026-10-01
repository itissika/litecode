import { cleanup, render, screen } from "@testing-library/react";
import type { IDockviewPanelProps } from "dockview-react";
import { afterEach, describe, expect, it } from "vitest";

import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { EdgeTab } from "./EdgeTab";

function edgeProps(component: string): IDockviewPanelProps {
  return {
    api: {
      id: component,
      title: component === "knowledge" ? "Knowledge" : "Explorer",
      component,
      location: { type: "edge", position: "left" },
      group: {
        api: {
          isCollapsed: () => true,
          onDidCollapsedChange: () => ({ dispose() {} }),
        },
      },
      onDidLocationChange: () => ({ dispose() {} }),
    },
  } as unknown as IDockviewPanelProps;
}

afterEach(() => {
  cleanup();
  useKnowledgeStore.setState({ issues: [], load: async () => {} });
});

describe("EdgeTab knowledge badge", () => {
  it("shows an error count and skips a disabled-target warning", () => {
    const issues: KnowledgeIssue[] = [
      {
        nodeId: "a",
        severity: "error",
        code: "missing_symbol",
        message: "missing",
      },
      {
        nodeId: "b",
        severity: "warning",
        code: "inactive_target",
        message: "inactive",
      },
    ];
    useKnowledgeStore.setState({ issues, load: async () => {} });
    render(<EdgeTab {...edgeProps("knowledge")} />);
    const badge = screen.getByLabelText("1 个节点有校验错误");
    expect(badge.textContent).toBe("1");
    expect(badge.className).toContain("is-error");
  });

  it("stays off other rail icons", () => {
    useKnowledgeStore.setState({
      issues: [
        {
          nodeId: "a",
          severity: "error",
          code: "missing_file",
          message: "missing",
        },
      ],
      load: async () => {},
    });
    render(<EdgeTab {...edgeProps("filetree")} />);
    expect(screen.queryByText("1")).toBeNull();
  });
});
