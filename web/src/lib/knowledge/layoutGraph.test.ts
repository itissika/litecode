import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import { layoutKnowledgeGraph } from "./layoutGraph";
import { validateKnowledge } from "./validate";

describe("layoutKnowledgeGraph", () => {
  const laid = layoutKnowledgeGraph(
    knowledgeFixture,
    validateKnowledge(knowledgeFixture),
  );

  it("places every node and skips dangling or self relations", () => {
    expect(laid.nodes).toHaveLength(knowledgeFixture.length);
    expect(
      laid.nodes.every(
        (node) => Number.isFinite(node.x) && Number.isFinite(node.y),
      ),
    ).toBe(true);
    expect(laid.edges.some((edge) => edge.source === edge.target)).toBe(false);
    expect(laid.edges.some((edge) => edge.target === "9999")).toBe(false);
    expect(laid.edges.some((edge) => edge.source === "17")).toBe(false);
  });

  it("draws a warning edge toward a disabled target and a solid edge for a real citation", () => {
    const sampling = laid.edges.find((edge) => edge.id === "19-18");
    expect(sampling?.warning).toBe(true);
    const sessionToSeq = laid.edges.find((edge) => edge.id === "1-2");
    expect(sessionToSeq?.warning).toBe(false);
  });
});
