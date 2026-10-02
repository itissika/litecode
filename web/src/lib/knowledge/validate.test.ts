import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import { fileMentionSource, mentionSource } from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";
import {
  groupIssues,
  knowledgeListAlert,
  knowledgeRailBadge,
  knowledgeTitleTone,
  missingFileIssues,
  symbolIssues,
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
        value: `uses ${mentionSource("seq")}`,
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
    expect(illegal.map((issue) => issue.message)).toEqual(["Key \"a/b\" is not valid."]);
  });

  it("flags a self citation and an unknown marker", () => {
    expect(
      codes(
        [node({ id: "loop", key: "loop", value: mentionSource("loop") })],
        "loop",
      ),
    ).toEqual(["self_relation"]);
    const nodes = [
      node({
        id: "session",
        key: "session",
        value: `${mentionSource("missing")} and ${mentionSource("seq")}`,
      }),
      node({ id: "seq", key: "seq" }),
    ];
    expect(codes(nodes, "session")).toEqual([
      "dangling_relation",
    ]);
  });

  it("warns when an enabled node cites a disabled or pending node", () => {
    const nodes = [
      node({
        id: "sampling",
        key: "sampling",
        value: `${mentionSource("temperature")} and ${mentionSource("knowledge")}`,
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
        value: `prose\n\`\`\`\n${mentionSource("seq")}\n\`\`\`\n\`${mentionSource("revert")}\``,
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

  it("reports a missing file only after the path has been checked", () => {
    const cited = node({
      id: "note",
      key: "note",
      value: `${fileMentionSource("src/a.rs")} ${fileMentionSource("../secret")}`,
    });
    expect(missingFileIssues([cited], {})).toEqual([
      expect.objectContaining({
        code: "missing_file",
        message: 'File "../secret" does not exist.',
        ref: "../secret",
      }),
    ]);
    expect(missingFileIssues([cited], { "src/a.rs": true })).toEqual([
      expect.objectContaining({ ref: "../secret" }),
    ]);
    expect(missingFileIssues([cited], { "src/a.rs": false })).toEqual([
      expect.objectContaining({
        message: 'File "src/a.rs" does not exist.',
        ref: "src/a.rs",
      }),
      expect.objectContaining({ ref: "../secret" }),
    ]);
    expect(validateKnowledge([cited]).some((issue) => issue.code === "missing_file")).toBe(
      false,
    );
  });

  it("reports a missing chain and a drifted chain", () => {
    const issues = symbolIssues([
      {
        nodeId: "note",
        file: "src/a.rs",
        symbol: "fn alpha",
        exists: false,
        ambiguous: false,
        drifted: false,
      },
      {
        nodeId: "note",
        file: "src/a.rs",
        symbol: "fn beta",
        exists: true,
        ambiguous: true,
        drifted: false,
      },
      {
        nodeId: "note",
        file: "src/a.rs",
        symbol: "fn gamma",
        exists: true,
        ambiguous: false,
        drifted: true,
      },
    ]);
    expect(issues.map((issue) => issue.code)).toEqual([
      "missing_symbol",
      "missing_symbol",
      "symbol_drift",
    ]);
    expect(issues[1]?.message).toContain("not unique");
    expect(issues[2]?.message).toContain("differs from HEAD");
    expect(issues[2]?.severity).toBe("warning");
  });

  it("allows a cycle", () => {
    const issues = validateKnowledge([
      node({ id: "session", key: "session", value: mentionSource("seq") }),
      node({ id: "seq", key: "seq", value: mentionSource("session") }),
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
