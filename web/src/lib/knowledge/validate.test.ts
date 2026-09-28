import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import type { KnowledgeIssue, KnowledgeNode } from "./types";
import {
  groupIssues,
  knowledgeListAlert,
  knowledgeTitleTone,
  validateKnowledge,
} from "./validate";

function node(
  partial: Partial<KnowledgeNode> & Pick<KnowledgeNode, "id" | "key">,
): KnowledgeNode {
  return {
    value: "",
    summary: "",
    relations: [],
    status: "enabled",
    path: `${partial.key.trim() || partial.id}.md`,
    x: null,
    y: null,
    w: null,
    h: null,
    ...partial,
  };
}

function codes(nodes: KnowledgeNode[], id: string): string[] {
  return validateKnowledge(nodes)
    .filter((issue) => issue.nodeId === id)
    .map((issue) => issue.code);
}

describe("validateKnowledge", () => {
  it("accepts a body citation that names a declared node", () => {
    const issues = validateKnowledge([
      node({
        id: "session",
        key: "session",
        value: "uses [[node : seq]]",
        relations: ["seq"],
      }),
      node({ id: "seq", key: "seq", value: "a number" }),
    ]);
    expect(issues).toEqual([]);
  });

  it("flags keys that differ only by surrounding space", () => {
    const issues = validateKnowledge([
      node({ id: "seq", key: "seq", path: "a/seq.md" }),
      node({ id: "b/seq.md", key: " seq ", path: "b/seq.md" }),
    ]);
    expect(issues.map((issue) => issue.code).sort()).toEqual([
      "duplicate_key",
      "duplicate_key",
    ]);
  });

  it("flags a missing declaration and an illegal key", () => {
    expect(
      codes([node({ id: "missing.md", key: "  ", path: "missing.md" })], "missing.md"),
    ).toEqual(["empty_key"]);
    const illegal = validateKnowledge([
      node({ id: "bad.md", key: "a/b", path: "bad.md" }),
    ]);
    expect(illegal.map((issue) => issue.message)).toEqual(["键「a/b」不合法"]);
  });

  it("flags a self citation and an unknown marker", () => {
    expect(
      codes(
        [node({ id: "loop", key: "loop", value: "[[node : loop]]", relations: ["loop"] })],
        "loop",
      ),
    ).toEqual(["self_relation"]);
    const nodes = [
      node({
        id: "session",
        key: "session",
        value: "[[node : missing]] and [[node : seq]]",
      }),
      node({ id: "seq", key: "seq" }),
    ];
    expect(codes(nodes, "session")).toEqual([
      "unregistered_marker",
      "unregistered_marker",
    ]);
  });

  it("warns when an enabled node cites a disabled or pending node", () => {
    const nodes = [
      node({
        id: "sampling",
        key: "sampling",
        value: "[[node : temperature]] and [[node : knowledge]]",
        relations: ["temperature", "knowledge"],
      }),
      node({ id: "temperature", key: "temperature", status: "disabled" }),
      node({ id: "knowledge", key: "knowledge", status: "pending" }),
    ];
    const issues = validateKnowledge(nodes).filter(
      (issue) => issue.nodeId === "sampling",
    );
    expect(issues.map((issue) => [issue.code, issue.severity, issue.ref])).toEqual([
      ["inactive_target", "warning", "temperature"],
      ["inactive_target", "warning", "knowledge"],
    ]);
  });

  it("warns when the file stem disagrees with the declaration", () => {
    const issues = validateKnowledge([
      node({ id: "seq", key: "seq", path: "内核/sequence.md" }),
    ]);
    expect(issues.map((issue) => issue.code)).toEqual(["filename_mismatch"]);
  });

  it("does not treat a marker inside code as a citation", () => {
    const issues = validateKnowledge([
      node({
        id: "note",
        key: "note",
        value: "prose\n```\n[[node : seq]]\n```\n`[[node : revert]]`",
      }),
    ]);
    expect(issues).toEqual([]);
  });

  it("leaves a bare double-bracket as text", () => {
    const issues = validateKnowledge([
      node({ id: "note", key: "note", value: "see [[seq]] and [[providers]]" }),
    ]);
    expect(issues).toEqual([]);
  });

  it("allows a cycle", () => {
    const issues = validateKnowledge([
      node({ id: "session", key: "session", value: "[[node : seq]]", relations: ["seq"] }),
      node({ id: "seq", key: "seq", value: "[[node : session]]", relations: ["session"] }),
    ]);
    expect(issues).toEqual([]);
  });
});

describe("knowledge fixture", () => {
  const issues = validateKnowledge(knowledgeFixture);

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
      new Set(["unregistered_marker", "self_relation", "inactive_target"]),
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

    expect(of("broken-marker")).toEqual(["unregistered_marker"]);
    expect(of("loopback")).toEqual(["self_relation"]);
    expect(of("sampling")).toEqual(["inactive_target"]);
    expect(of("session")).toEqual([]);
    expect(of("temperature")).toEqual([]);
    expect(of("knowledge")).toEqual([]);
  });
});

describe("knowledgeTitleTone", () => {
  const issuesByNode = groupIssues(validateKnowledge(knowledgeFixture));
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
      code: "unknown_marker",
      message: "正文标识「missing」不存在",
    };
    expect(knowledgeTitleTone([fault], "disabled")).toBe("error");
  });
});

describe("knowledgeListAlert", () => {
  const issuesByNode = groupIssues(validateKnowledge(knowledgeFixture));
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
