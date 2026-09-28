import { describe, expect, it } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "./fixture";
import { layoutKnowledgeGraph } from "./layoutGraph";
import type { KnowledgeNode } from "./types";
import { validateKnowledge } from "./validate";

function node(
  partial: Partial<KnowledgeNode> & Pick<KnowledgeNode, "id" | "key">,
): KnowledgeNode {
  return {
    value: "",
    relations: [],
    status: "enabled",
    ...partial,
  };
}

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

  it("classifies relation edges by registration vs inactive target", () => {
    const sampling = laid.edges.find((edge) => edge.id === "19-18");
    expect(sampling?.variant).toBe("inactive");
    const sessionToSeq = laid.edges.find((edge) => edge.id === "1-2");
    expect(sessionToSeq?.variant).toBe("solid");
    const draftLink = laid.edges.find((edge) => edge.id === "21-20");
    expect(draftLink?.variant).toBe("inactive");
  });

  it("nests folder frames and keeps members parent-relative", () => {
    const laidWithFolders = layoutKnowledgeGraph(
      knowledgeFixture,
      validateKnowledge(knowledgeFixture),
      knowledgeFolderFixture,
    );
    expect(laidWithFolders.nodes).toHaveLength(knowledgeFixture.length);
    const context = laidWithFolders.folders.find((folder) => folder.folderId === 2);
    const kernel = laidWithFolders.folders.find((folder) => folder.folderId === 1);
    const item = laidWithFolders.nodes.find((node) => node.nodeId === 4);
    const dockview = laidWithFolders.nodes.find((node) => node.nodeId === 12);
    expect(kernel?.parentId).toBeNull();
    expect(context?.parentId).toBe("folder:1");
    expect(item?.parentId).toBe("folder:2");
    expect(item && item.y).toBeGreaterThanOrEqual(28);
    expect(dockview?.parentId).toBeNull();
    expect(kernel).toBeDefined();
    expect(context).toBeDefined();
    expect(kernel!.width).toBeGreaterThan(context!.width);
  });

  it("marks a registered-but-unused relation as unused", () => {
    const nodes = [
      node({ id: 1, key: "host", value: "plain", relations: [2] }),
      node({ id: 2, key: "peer" }),
    ];
    const edges = layoutKnowledgeGraph(nodes, validateKnowledge(nodes)).edges;
    expect(edges).toHaveLength(1);
    expect(edges[0]?.variant).toBe("unused");
  });
});
