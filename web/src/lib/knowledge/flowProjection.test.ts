import { describe, expect, it } from "vitest";

import { mentionSource } from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";
import {
  canvasAlertKey,
  easeOutCubic,
  knowledgeContentSignature,
  edgeOpacity,
  focusSpotlightIds,
  knowledgeStructureKey,
  mentionKeysOf,
  patchById,
  patchEdgesForFocus,
  reconcileEdges,
  relationSignature,
  replaceOneRecord,
  retainLeaving,
  reuseIssueGroups,
} from "./flowProjection";
import { knowledgeEdgeZIndex } from "../../components/knowledge/KnowledgeGraph";
import { projectListedNodes } from "../../stores/knowledgeStore";

function node(id: string, extra: Partial<KnowledgeNode> = {}): KnowledgeNode {
  return {
    id,
    key: id,
    value: "",
    summary: "",
    relations: [],
    status: "enabled",
    x: null,
    y: null,
    w: null,
    h: null,
    path: `${id}.md`,
    ...extra,
  };
}

describe("retainLeaving", () => {
  const leaving = (item: { id: string; leaving?: boolean }) => item.leaving === true;
  const mark = <T extends { id: string }>(item: T) => ({ ...item, leaving: true });

  it("marks a removed card once and keeps it beside the live ones", () => {
    const prev = [{ id: "a" }, { id: "b" }];
    const next = [{ id: "a" }];
    const first = retainLeaving(prev, next, leaving, mark);
    expect(first.started).toEqual(["b"]);
    expect(first.cancelled).toEqual([]);
    expect(first.items.map((item) => item.id)).toEqual(["a", "b"]);
    const second = retainLeaving(first.items, next, leaving, mark);
    expect(second.started).toEqual([]);
    expect(second.items[1]).toBe(first.items[1]);
  });

  it("drops the ghost when the same id comes back", () => {
    const prev = [{ id: "b", leaving: true }];
    const next = [{ id: "b" }];
    const returned = retainLeaving(prev, next, leaving, mark);
    expect(returned.cancelled).toEqual(["b"]);
    expect(returned.items).toEqual([{ id: "b" }]);
  });
});

describe("knowledgeContentSignature", () => {
  it("treats a geometry rewrite's trailing whitespace as the same body", () => {
    expect(knowledgeContentSignature("note", "hello")).toBe(
      knowledgeContentSignature("note", "hello\n"),
    );
    expect(knowledgeContentSignature(" note ", "hello")).toBe(
      knowledgeContentSignature("note", "hello"),
    );
  });

  it("changes when the summary or the body text changes", () => {
    const base = knowledgeContentSignature("note", "hello");
    expect(knowledgeContentSignature("other", "hello")).not.toBe(base);
    expect(knowledgeContentSignature("note", "hello!")).not.toBe(base);
  });
});

describe("easeOutCubic", () => {
  it("starts at rest, ends at the target, and is ahead of linear halfway", () => {
    expect(easeOutCubic(0)).toBe(0);
    expect(easeOutCubic(1)).toBe(1);
    expect(easeOutCubic(0.5)).toBeGreaterThan(0.5);
  });
});

describe("patchById", () => {
  it("replaces only the named card", () => {
    const cards = [
      { id: "a", open: false },
      { id: "b", open: false },
      { id: "c", open: false },
    ];
    const next = patchById(cards, new Set(["b"]), (card) => ({ ...card, open: true }));
    expect(next[0]).toBe(cards[0]);
    expect(next[2]).toBe(cards[2]);
    expect(next[1]).toEqual({ id: "b", open: true });
    expect(next[1]).not.toBe(cards[1]);
  });

  it("returns the same array when the recipe keeps every object", () => {
    const cards = [{ id: "a" }, { id: "b" }];
    expect(patchById(cards, new Set(["a"]), (card) => card)).toBe(cards);
  });
});

describe("focusSpotlightIds", () => {
  it("includes citation-linked neighbours of the focused card", () => {
    const nodes = [
      node("a", { key: "alpha", relations: ["beta"] }),
      node("b", { key: "beta", relations: [] }),
      node("c", { key: "gamma", relations: ["alpha"] }),
      node("d"),
    ];
    const spotlight = focusSpotlightIds("a", nodes);
    expect(spotlight).toEqual(new Set(["a", "b", "c"]));
    expect(edgeOpacity({ source: "b", target: "c" }, spotlight)).toBe(1);
    expect(edgeOpacity({ source: "c", target: "d" }, spotlight)).toBe(0.16);
  });
});

describe("patchEdgesForFocus", () => {
  const nodes = [
    node("a", { key: "alpha", relations: ["beta"] }),
    node("b", { key: "beta" }),
    node("c"),
    node("d"),
  ];
  const edges = [
    { id: "a-b", source: "a", target: "b", data: { opacity: 1, variant: "solid" }, zIndex: 5 },
    { id: "c-d", source: "c", target: "d", data: { opacity: 1, variant: "solid" }, zIndex: 5 },
  ];

  it("keeps spotlight edges bright and lifts their z-index", () => {
    const next = patchEdgesForFocus(edges, null, "a", nodes, knowledgeEdgeZIndex);
    expect(next[0].data?.opacity).toBe(1);
    expect(next[0].zIndex).toBeGreaterThan(5);
    expect(next[1]).not.toBe(edges[1]);
    expect(next[1].data?.opacity).toBe(0.16);
  });

  it("leaves an exiting edge untouched when focus moves", () => {
    const exiting = {
      id: "a-b",
      source: "a",
      target: "b",
      data: { opacity: 0, leaving: true },
      zIndex: 5,
    };
    const next = patchEdgesForFocus([exiting], null, "a", nodes, knowledgeEdgeZIndex);
    expect(next[0]).toBe(exiting);
  });

  it("keeps edges that stay dim when focus moves to another card", () => {
    const dimmed = patchEdgesForFocus(edges, null, "a", nodes, knowledgeEdgeZIndex);
    const moved = patchEdgesForFocus(dimmed, "a", "z", nodes, knowledgeEdgeZIndex);
    expect(moved[1]).toBe(dimmed[1]);
    expect(moved[0]).not.toBe(dimmed[0]);
    expect(moved[0].data?.opacity).toBe(0.16);
  });
});

describe("reconcileEdges", () => {
  it("keeps edges whose citation did not change", () => {
    const prev = [
      {
        id: "a-b",
        source: "a",
        target: "b",
        data: { variant: "solid", stroke: "#777", opacity: 1 },
      },
      {
        id: "a-c",
        source: "a",
        target: "c",
        data: { variant: "solid", stroke: "#777", opacity: 1 },
      },
    ];
    const laid = [
      { id: "a-b", source: "a", target: "b", variant: "solid" as const },
      { id: "a-d", source: "a", target: "d", variant: "solid" as const },
    ];
    const next = reconcileEdges(
      prev,
      laid,
      null,
      "#777",
      (edge, opacity, stroke, zIndex) => ({
        id: edge.id,
        source: edge.source,
        target: edge.target,
        zIndex,
        data: { variant: edge.variant, stroke, opacity },
      }),
      knowledgeEdgeZIndex,
    );
    expect(next[0]).toBe(prev[0]);
    expect(next[1]).not.toBe(prev[1]);
    expect(next[1].id).toBe("a-d");
  });
});

describe("corpus scope", () => {
  it("ignores summary when classifying structure and relations", () => {
    const before = [node("a", { summary: "one", relations: ["b"] }), node("b")];
    const after = [node("a", { summary: "two", relations: ["b"] }), node("b")];
    expect(knowledgeStructureKey(before, [])).toBe(knowledgeStructureKey(after, []));
    expect(relationSignature(before)).toBe(relationSignature(after));
    expect(mentionKeysOf(before)).toBe(mentionKeysOf(after));
    expect(canvasAlertKey(before, [])).toBe(canvasAlertKey(after, []));
  });

  it("keeps the other node and its issue list when one summary changes", () => {
    const broken = node("a", {
      value: mentionSource("missing"),
      relations: ["missing"],
    });
    const other = node("b", { summary: "kept" });
    const first = projectListedNodes(
      { nodes: [], issuesByNode: new Map(), filePresence: {}, folders: [] },
      [broken, other],
    );
    const edited = { ...broken, summary: "edited" };
    const second = projectListedNodes(
      {
        nodes: first.nodes,
        issuesByNode: first.issuesByNode,
        filePresence: {},
        folders: [],
      },
      [edited, other],
    );
    expect(second.byId.get("b")).toBe(other);
    expect(second.nodes[1]).toBe(other);
    expect(second.issuesByNode.get("a")).toBe(first.issuesByNode.get("a"));
    expect(second.issuesByNode.get("b")).toBeUndefined();
  });

  it("reuses an issue list with the same contents", () => {
    const list: KnowledgeIssue[] = [
      {
        nodeId: "a",
        severity: "error",
        code: "dangling_relation",
        message: "missing",
        ref: "missing",
      },
    ];
    const previous = new Map([["a", list]]);
    const next = new Map([
      [
        "a",
        [
          {
            nodeId: "a",
            severity: "error" as const,
            code: "dangling_relation" as const,
            message: "missing",
            ref: "missing",
          },
        ],
      ],
    ]);
    expect(reuseIssueGroups(previous, next)).toBe(previous);
  });

  it("replaces one geometry record and leaves the others", () => {
    const a = node("a", { x: 1, y: 2 });
    const b = node("b", { x: 3, y: 4 });
    const moved = { ...a, x: 10, y: 12 };
    const index = new Map([
      ["a", a],
      ["b", b],
    ]);
    const replaced = replaceOneRecord([a, b], index, moved);
    expect(replaced.records[1]).toBe(b);
    expect(replaced.index.get("b")).toBe(b);
    expect(replaced.index.get("a")).toBe(moved);
    expect(replaced.records[0]).toBe(moved);
  });
});
