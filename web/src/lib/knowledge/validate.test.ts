import { describe, expect, it } from "vitest";

import { knowledgeFixture, knowledgeFixtureIssues } from "../../test/fixtures/knowledge";
import type { KnowledgeIssue } from "./types";
import {
  groupIssues,
  knowledgeAttentionWarnings,
  knowledgeListAlert,
  knowledgeRailBadge,
  knowledgeTitleTone,
} from "./validate";

describe("knowledge fixture", () => {
  const issues = knowledgeFixtureIssues;

  it("covers a readable slice of LiteCode concepts, including the deliberate faults", () => {
    expect(knowledgeFixture.length).toBeGreaterThanOrEqual(15);
    expect(knowledgeFixture.length).toBeLessThanOrEqual(25);
    const byKey = new Map(knowledgeFixture.map((item) => [item.key, item]));
    expect(byKey.get("temperature")?.status).toBe("disabled");
    expect(byKey.get("knowledge")?.status).toBe("pending");
    expect(byKey.get("session")?.relations).toEqual(
      expect.arrayContaining(["seq", "revert"]),
    );
    expect(byKey.get("seq")?.relations).toEqual(
      expect.arrayContaining(["session", "revert"]),
    );
    expect(byKey.get("session")?.id).toBe("session");
    expect(byKey.get("item")?.folderId).toBe("内核/上下文");
    expect(byKey.get("dockview")?.folderId).toBeNull();

    const present = new Set(issues.map((issue) => issue.code));
    expect(present).toEqual(
      new Set(["dangling_relation", "self_relation", "inactive_target"]),
    );
  });

  it("keeps each planted fault on the node that demonstrates it", () => {
    const of = (key: string) =>
      issues
        .filter(
          (issue) =>
            issue.nodeId === knowledgeFixture.find((item) => item.key === key)?.id,
        )
        .map((issue) => issue.code);

    expect(of("broken-marker")).toEqual(["dangling_relation"]);
    expect(of("loopback")).toEqual(["self_relation"]);
    expect(of("sampling")).toEqual(["inactive_target"]);
    expect(of("session")).toEqual([]);
    expect(of("temperature")).toEqual([]);
    expect(of("knowledge")).toEqual([]);
  });
});

describe("knowledgeTitleTone", () => {
  const issuesByNode = groupIssues(knowledgeFixtureIssues);
  const nodeByKey = new Map(knowledgeFixture.map((n) => [n.key, n]));

  function toneFor(key: string) {
    const item = nodeByKey.get(key)!;
    return knowledgeTitleTone(issuesByNode.get(item.id) ?? [], item.status);
  }

  it("maps faults to error, status to disabled / pending", () => {
    expect(toneFor("broken-marker")).toBe("error");
    expect(toneFor("temperature")).toBe("disabled");
    expect(toneFor("knowledge")).toBe("pending");
    expect(toneFor("session")).toBeNull();
    expect(toneFor("sampling")).toBeNull();
  });

  it("keeps the fault tone on a disabled node", () => {
    const fault: KnowledgeIssue = {
      nodeId: "temperature",
      severity: "error",
      code: "dangling_relation",
      message: "Citation \"missing\" does not exist.",
    };
    expect(knowledgeTitleTone([fault], "disabled")).toBe("error");
  });
});

describe("knowledgeRailBadge", () => {
  const error = (nodeId: string): KnowledgeIssue => ({
    nodeId,
    severity: "error",
    code: "missing_file",
    message: "missing",
  });
  const drift = (nodeId: string): KnowledgeIssue => ({
    nodeId,
    severity: "warning",
    code: "symbol_drift",
    message: "drift",
  });
  const inactive = (nodeId: string): KnowledgeIssue => ({
    nodeId,
    severity: "warning",
    code: "inactive_target",
    message: "inactive",
  });

  it("counts nodes with errors ahead of warnings", () => {
    expect(knowledgeRailBadge([error("a"), error("a"), drift("b")])).toEqual({
      tone: "error",
      count: 1,
    });
  });

  it("counts warning nodes when nothing is an error", () => {
    expect(knowledgeRailBadge([drift("a"), inactive("a"), drift("b")])).toEqual({
      tone: "warning",
      count: 2,
    });
  });

  it("ignores a citation of a disabled node on its own", () => {
    expect(knowledgeRailBadge([inactive("a")])).toBeNull();
    expect(knowledgeRailBadge([])).toBeNull();
  });
});

describe("knowledgeListAlert", () => {
  const issuesByNode = groupIssues(knowledgeFixtureIssues);
  const nodeByKey = new Map(knowledgeFixture.map((n) => [n.key, n]));

  function alertFor(key: string) {
    const item = nodeByKey.get(key)!;
    return knowledgeListAlert(issuesByNode.get(item.id) ?? [], item.status);
  }

  it("flags reference faults and pending review", () => {
    expect(alertFor("broken-marker")).toBe("red");
    expect(alertFor("knowledge")).toBe("amber");
    expect(alertFor("session")).toBeNull();
    expect(alertFor("sampling")).toBeNull();
    expect(alertFor("temperature")).toBeNull();
  });
});

describe("knowledgeAttentionWarnings", () => {
  const drift: KnowledgeIssue = {
    nodeId: "a",
    severity: "warning",
    code: "symbol_drift",
    message: "drift",
  };
  const mismatch: KnowledgeIssue = {
    nodeId: "a",
    severity: "warning",
    code: "filename_mismatch",
    message: "mismatch",
  };
  const inactive: KnowledgeIssue = {
    nodeId: "a",
    severity: "warning",
    code: "inactive_target",
    message: "inactive",
  };
  const error: KnowledgeIssue = {
    nodeId: "a",
    severity: "error",
    code: "missing_file",
    message: "missing",
  };

  it("keeps attention warnings and drops errors and inactive citations", () => {
    expect(
      knowledgeAttentionWarnings([mismatch, inactive, error, drift]),
    ).toEqual([mismatch, drift]);
  });

  it("stays empty for a node with no attention warnings", () => {
    expect(knowledgeAttentionWarnings([inactive, error])).toEqual([]);
    expect(knowledgeAttentionWarnings([])).toEqual([]);
  });
});
