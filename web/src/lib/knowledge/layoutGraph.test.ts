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
    summary: "",
    relations: [],
    status: "enabled",
    path: `${partial.key}.md`,
    x: null,
    y: null,
    w: null,
    h: null,
    ...partial,
  };
}

describe("layoutKnowledgeGraph", () => {
  const laid = layoutKnowledgeGraph(
    knowledgeFixture,
    validateKnowledge(knowledgeFixture),
  );

  it("places every node and skips self relations", () => {
    expect(laid.nodes).toHaveLength(knowledgeFixture.length);
    expect(
      laid.nodes.every(
        (item) => Number.isFinite(item.x) && Number.isFinite(item.y),
      ),
    ).toBe(true);
    expect(laid.edges.some((edge) => edge.source === edge.target)).toBe(false);
    expect(laid.edges.some((edge) => edge.source === "loopback")).toBe(false);
  });

  it("classifies a citation of a disabled node as inactive", () => {
    const sampling = laid.edges.find((edge) => edge.id === "sampling-temperature");
    expect(sampling?.variant).toBe("inactive");
    const sessionToSeq = laid.edges.find((edge) => edge.id === "session-seq");
    expect(sessionToSeq?.variant).toBe("solid");
  });

  it("nests folder frames and keeps members parent-relative", () => {
    const laidWithFolders = layoutKnowledgeGraph(
      knowledgeFixture,
      validateKnowledge(knowledgeFixture),
      knowledgeFolderFixture,
    );
    expect(laidWithFolders.nodes).toHaveLength(knowledgeFixture.length);
    const context = laidWithFolders.folders.find(
      (folder) => folder.folderId === "内核/上下文",
    );
    const kernel = laidWithFolders.folders.find((folder) => folder.folderId === "内核");
    const item = laidWithFolders.nodes.find((entry) => entry.nodeId === "item");
    const dockview = laidWithFolders.nodes.find(
      (entry) => entry.nodeId === "dockview",
    );
    expect(kernel?.parentId).toBeNull();
    expect(context?.parentId).toBe("folder:内核");
    expect(item?.parentId).toBe("folder:内核/上下文");
    expect(item && item.y).toBeGreaterThanOrEqual(28);
    expect(dockview?.parentId).toBeNull();
    expect(kernel).toBeDefined();
    expect(context).toBeDefined();
    expect(kernel!.width).toBeGreaterThan(context!.width);
  });

  it("draws a solid edge for a resolved body citation", () => {
    const nodes = [
      node({
        id: "host",
        key: "host",
        value: '[@ id="peer" label="peer"]',
        relations: ["peer"],
      }),
      node({ id: "peer", key: "peer" }),
    ];
    const edges = layoutKnowledgeGraph(nodes, validateKnowledge(nodes)).edges;
    expect(edges).toHaveLength(1);
    expect(edges[0]?.variant).toBe("solid");
    expect(edges[0]?.id).toBe("host-peer");
  });
});
