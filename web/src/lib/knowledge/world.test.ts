import { describe, expect, it } from "vitest";

import {
  flowToWorld,
  worldFromFlowNode,
  worldToFlow,
  worldWritesForPositionChanges,
  type FlowPointNode,
} from "./world";

function card(
  id: string,
  position: { x: number; y: number },
  parentId?: string,
): FlowPointNode {
  return { id, type: "knowledge", parentId, position };
}

function folder(
  id: string,
  position: { x: number; y: number },
  parentId?: string,
): FlowPointNode {
  return { id, type: "knowledgeFolder", parentId, position };
}

describe("world coordinates", () => {
  it("converts a nested flow position to and from world coordinates", () => {
    const world = flowToWorld({ x: 16, y: 44 }, [
      { x: 20, y: 30 },
      { x: 100, y: 200 },
    ]);
    expect(world).toEqual({ x: 136, y: 274 });
    expect(worldToFlow(world, { x: 120, y: 230 })).toEqual({ x: 16, y: 44 });
    expect(worldToFlow(world, null)).toEqual(world);
  });

  it("writes a card's world position when its drag ends", () => {
    const fitted = [
      folder("folder:box", { x: 10, y: 20 }),
      card("a", { x: 16.4, y: 44.2 }, "folder:box"),
    ];
    expect(worldFromFlowNode(fitted[1], new Map(fitted.map((node) => [node.id, node])))).toEqual({
      x: 26.4,
      y: 64.2,
    });
    expect(
      worldWritesForPositionChanges(
        [{ type: "position", id: "a", dragging: false, position: { x: 16.4, y: 44.2 } }],
        fitted,
      ),
    ).toEqual([{ id: "a", x: 26, y: 64 }]);
  });

  it("writes every card inside a folder when the folder drag ends", () => {
    const fitted = [
      folder("folder:a", { x: 10, y: 0 }),
      folder("folder:a/b", { x: 16, y: 44 }, "folder:a"),
      card("outer", { x: 16, y: 44 }, "folder:a"),
      card("inner", { x: 16, y: 44 }, "folder:a/b"),
    ];
    expect(
      worldWritesForPositionChanges(
        [{ type: "position", id: "folder:a", dragging: false, position: { x: 10, y: 0 } }],
        fitted,
      ),
    ).toEqual([
      { id: "outer", x: 26, y: 44 },
      { id: "inner", x: 42, y: 88 },
    ]);
  });

  it("ignores a drag that is still moving", () => {
    const fitted = [card("a", { x: 8, y: 8 })];
    expect(
      worldWritesForPositionChanges(
        [{ type: "position", id: "a", dragging: true, position: { x: 8, y: 8 } }],
        fitted,
      ),
    ).toEqual([]);
  });
});
