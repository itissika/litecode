import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import type { KnowledgeNode } from "./types";
import { groupIssues, knowledgeListAlert, validateKnowledge } from "./validate";

function node(partial: Partial<KnowledgeNode> & Pick<KnowledgeNode, "id" | "key">): KnowledgeNode {
  return {
    value: "",
    relations: [],
    status: "enabled",
    ...partial,
  };
}

function codes(nodes: KnowledgeNode[], id: number): string[] {
  return validateKnowledge(nodes)
    .filter((issue) => issue.nodeId === id)
    .map((issue) => issue.code);
}

describe("validateKnowledge", () => {
  it("accepts a registered citation", () => {
    const issues = validateKnowledge([
      node({ id: 1, key: "session", value: "uses [[seq]]", relations: [2] }),
      node({ id: 2, key: "seq", value: "a number" }),
    ]);
    expect(issues).toEqual([]);
  });

  it("flags duplicate ids and keys that differ only by surrounding space", () => {
    const issues = validateKnowledge([
      node({ id: 1, key: "seq" }),
      node({ id: 1, key: " seq " }),
    ]);
    expect(issues.map((issue) => issue.code).sort()).toEqual([
      "duplicate_id",
      "duplicate_id",
      "duplicate_key",
      "duplicate_key",
    ]);
  });

  it("flags an empty key", () => {
    expect(codes([node({ id: 1, key: "  " })], 1)).toEqual(["empty_key"]);
  });

  it("flags a missing relation target and a self relation", () => {
    expect(
      codes([node({ id: 4, key: "loop", relations: [4, 9] })], 4),
    ).toEqual(["self_relation", "dangling_relation"]);
  });

  it("flags an unknown marker, and a known marker that was not declared", () => {
    const nodes = [
      node({ id: 1, key: "session", value: "[[missing]] and [[seq]]" }),
      node({ id: 2, key: "seq" }),
    ];
    expect(codes(nodes, 1)).toEqual(["unknown_marker", "unregistered_marker"]);
  });

  it("warns when a declared relation is unused or points at an inactive node", () => {
    const nodes = [
      node({
        id: 1,
        key: "sampling",
        value: "plain text",
        relations: [2, 3],
      }),
      node({ id: 2, key: "temperature", status: "disabled" }),
      node({ id: 3, key: "knowledge", status: "pending" }),
    ];
    const issues = validateKnowledge(nodes).filter(
      (issue) => issue.nodeId === 1,
    );
    expect(issues.map((issue) => [issue.code, issue.severity, issue.ref])).toEqual([
      ["unused_relation", "warning", "temperature"],
      ["inactive_target", "warning", "temperature"],
      ["unused_relation", "warning", "knowledge"],
      ["inactive_target", "warning", "knowledge"],
    ]);
  });

  it("does not treat a marker inside code as a citation", () => {
    const issues = validateKnowledge([
      node({
        id: 1,
        key: "note",
        value: "prose\n```\n[[seq]]\n```\n`[[revert]]`",
      }),
    ]);
    expect(issues).toEqual([]);
  });

  it("allows a cycle", () => {
    const issues = validateKnowledge([
      node({ id: 1, key: "session", value: "[[seq]]", relations: [2] }),
      node({ id: 2, key: "seq", value: "[[session]]", relations: [1] }),
    ]);
    expect(issues).toEqual([]);
  });
});

describe("knowledge fixture", () => {
  const issues = validateKnowledge(knowledgeFixture);

  it("covers a readable slice of LiteCode concepts, including the deliberate faults", () => {
    expect(knowledgeFixture.length).toBeGreaterThanOrEqual(15);
    expect(knowledgeFixture.length).toBeLessThanOrEqual(25);
    const byKey = new Map(knowledgeFixture.map((node) => [node.key, node]));
    expect(byKey.get("temperature")?.status).toBe("disabled");
    expect(byKey.get("knowledge")?.status).toBe("pending");
    expect(byKey.get("session")?.relations).toContain(
      byKey.get("seq")?.id,
    );
    expect(byKey.get("seq")?.relations).toContain(byKey.get("session")?.id);

    const present = new Set(issues.map((issue) => issue.code));
    expect(present).toEqual(
      new Set([
        "unknown_marker",
        "unregistered_marker",
        "dangling_relation",
        "self_relation",
        "unused_relation",
        "inactive_target",
      ]),
    );
  });

  it("keeps each planted fault on the node that demonstrates it", () => {
    const of = (key: string) =>
      issues
        .filter(
          (issue) =>
            issue.nodeId === knowledgeFixture.find((node) => node.key === key)?.id,
        )
        .map((issue) => issue.code);

    expect(of("broken-marker")).toEqual(["unknown_marker"]);
    expect(of("loose-ref")).toEqual(["unregistered_marker"]);
    expect(of("dangling")).toEqual(["dangling_relation"]);
    expect(of("loopback")).toEqual(["self_relation"]);
    expect(of("sampling")).toEqual(["inactive_target"]);
    expect(of("draft-link").sort()).toEqual([
      "inactive_target",
      "unused_relation",
    ]);
    expect(of("session")).toEqual([]);
    expect(of("temperature")).toEqual([]);
  });
});

describe("knowledgeListAlert", () => {
  const issuesByNode = groupIssues(validateKnowledge(knowledgeFixture));
  const nodeByKey = new Map(knowledgeFixture.map((n) => [n.key, n]));

  function alertFor(key: string) {
    const node = nodeByKey.get(key)!;
    return knowledgeListAlert(issuesByNode.get(node.id) ?? [], node.status);
  }

  it("flags reference faults and pending review", () => {
    expect(alertFor("broken-marker")).toBe("red");
    expect(alertFor("draft-link")).toBe("red");
    expect(alertFor("knowledge")).toBe("amber");
    expect(alertFor("session")).toBeNull();
    expect(alertFor("sampling")).toBeNull();
    expect(alertFor("temperature")).toBeNull();
  });
});
