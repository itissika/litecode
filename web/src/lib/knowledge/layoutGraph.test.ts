import { describe, expect, it } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../test/fixtures/knowledge";
import {
  layoutKnowledgeGraph,
  snapKnowledgeCoord,
  KNOWLEDGE_FOLDER_HEADER,
  KNOWLEDGE_FOLDER_PAD,
  KNOWLEDGE_GRID,
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_WIDTH,
} from "./layoutGraph";
import type { KnowledgeNode } from "./types";
import { knowledgeFixtureIssues as fixtureIssues } from "../../test/fixtures/knowledge";

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
    fixtureIssues,
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
    for (const item of laid.nodes) {
      expect(item.x % KNOWLEDGE_GRID).toBe(0);
      expect(item.y % KNOWLEDGE_GRID).toBe(0);
    }
  });

  it("classifies a citation of a disabled node as inactive", () => {
    const sampling = laid.edges.find((edge) => edge.id === "sampling-temperature");
    expect(sampling?.variant).toBe("inactive");
    const sessionToSeq = laid.edges.find((edge) => edge.id === "session-seq");
    expect(sessionToSeq?.variant).toBe("solid");
  });

  it("nests folder frames around the nodes inside them", () => {
    const laidWithFolders = layoutKnowledgeGraph(
      knowledgeFixture,
      fixtureIssues,
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
    expect(context).toBeDefined();
    expect(item!.x).toBeGreaterThanOrEqual(context!.x);
    expect(item!.y).toBeGreaterThanOrEqual(context!.y);
    expect(item!.x + KNOWLEDGE_NODE_WIDTH).toBeLessThanOrEqual(context!.x + context!.width + 0.5);
    expect(item!.y + KNOWLEDGE_NODE_HEIGHT).toBeLessThanOrEqual(context!.y + context!.height + 0.5);
    expect(dockview?.parentId).toBeNull();
    expect(kernel).toBeDefined();
    expect(kernel!.width).toBeGreaterThan(context!.width);
  });

  it("keeps a saved position as world coordinates and derives the folder frame", () => {
    const laidWithSaved = layoutKnowledgeGraph(
      [
        node({ id: "inner", key: "inner", folderId: "box", x: 500, y: 640 }),
        node({ id: "beside", key: "beside", folderId: "box", x: 300, y: 100 }),
      ],
      [],
      [{ id: "box", name: "box", parentId: null }],
    );
    const inner = laidWithSaved.nodes.find((entry) => entry.id === "inner");
    const beside = laidWithSaved.nodes.find((entry) => entry.id === "beside");
    const frame = laidWithSaved.folders[0];
    expect(inner).toMatchObject({ x: 500, y: 640, parentId: "folder:box" });
    expect(beside).toMatchObject({ x: 300, y: 100 });
    expect(frame?.x).toBe(300 - 16);
    expect(frame?.y).toBe(100 - (28 + 16));
    expect(frame?.width).toBe(500 + KNOWLEDGE_NODE_WIDTH - 300 + KNOWLEDGE_FOLDER_PAD * 2);
    expect(frame?.height).toBe(
      KNOWLEDGE_FOLDER_HEADER + (640 + KNOWLEDGE_NODE_HEIGHT - 100) + KNOWLEDGE_FOLDER_PAD * 2,
    );
  });

  it("places an unsaved node beside saved world coordinates", () => {
    const laidWithFree = layoutKnowledgeGraph(
      [
        node({ id: "pinned", key: "pinned", folderId: "box", x: 100, y: 80 }),
        node({ id: "free", key: "free", folderId: "box" }),
      ],
      [],
      [{ id: "box", name: "box", parentId: null }],
    );
    const pinned = laidWithFree.nodes.find((entry) => entry.id === "pinned");
    const free = laidWithFree.nodes.find((entry) => entry.id === "free");
    expect(pinned).toMatchObject({ x: 100, y: 80 });
    expect(free).toMatchObject({
      x: snapKnowledgeCoord(100 + KNOWLEDGE_NODE_WIDTH + 28),
      y: snapKnowledgeCoord(80),
    });
    expect(free!.x % KNOWLEDGE_GRID).toBe(0);
    expect(free!.y % KNOWLEDGE_GRID).toBe(0);
  });

  it("draws a solid edge for a resolved body citation", () => {
    const nodes = [
      node({
        id: "host",
        key: "host",
        value: '[@ key="peer"]',
        relations: ["peer"],
      }),
      node({ id: "peer", key: "peer" }),
    ];
    const edges = layoutKnowledgeGraph(nodes, []).edges;
    expect(edges).toHaveLength(1);
    expect(edges[0]?.variant).toBe("solid");
    expect(edges[0]?.id).toBe("host-peer");
  });
});
