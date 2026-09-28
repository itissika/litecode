import { describe, expect, it } from "vitest";

import { fitKnowledgeFolders, type FitNode } from "./fitFolders";
import { KNOWLEDGE_FOLDER_HEADER, KNOWLEDGE_FOLDER_PAD } from "./layoutGraph";

function folder(
  id: string,
  position: { x: number; y: number },
  size: { w: number; h: number },
  parentId?: string,
): FitNode {
  return {
    id,
    type: "knowledgeFolder",
    parentId,
    position,
    width: size.w,
    height: size.h,
    style: { width: size.w, height: size.h },
  };
}

function card(
  id: string,
  position: { x: number; y: number },
  parentId: string,
  size = { w: 100, h: 40 },
): FitNode {
  return {
    id,
    type: "knowledge",
    parentId,
    position,
    width: size.w,
    height: size.h,
  };
}

describe("fitKnowledgeFolders", () => {
  it("grows and shifts the direct parent so padding stays fixed", () => {
    const fitted = fitKnowledgeFolders([
      folder("folder:1", { x: 0, y: 0 }, { w: 200, h: 120 }),
      card("a", { x: -30, y: 80 }, "folder:1", { w: 100, h: 40 }),
    ]);
    const frame = fitted.find((node) => node.id === "folder:1")!;
    const child = fitted.find((node) => node.id === "a")!;
    expect(child.position.x).toBe(KNOWLEDGE_FOLDER_PAD);
    expect(child.position.y).toBe(KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD);
    expect(frame.width).toBe(100 + KNOWLEDGE_FOLDER_PAD * 2);
    expect(frame.height).toBe(
      KNOWLEDGE_FOLDER_HEADER + 40 + KNOWLEDGE_FOLDER_PAD * 2,
    );
    expect(frame.position.x).toBe(-30 - KNOWLEDGE_FOLDER_PAD);
    expect(frame.position.y).toBe(80 - (KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD));
  });

  it("shrinks when children pull inward", () => {
    const wide = fitKnowledgeFolders([
      folder("folder:1", { x: 0, y: 0 }, { w: 400, h: 300 }),
      card("a", { x: KNOWLEDGE_FOLDER_PAD, y: KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD }, "folder:1"),
    ]);
    const frame = wide.find((node) => node.id === "folder:1")!;
    expect(frame.width).toBe(100 + KNOWLEDGE_FOLDER_PAD * 2);
    expect(frame.height).toBe(KNOWLEDGE_FOLDER_HEADER + 40 + KNOWLEDGE_FOLDER_PAD * 2);
  });

  it("bubbles a nested folder resize to the ancestor", () => {
    const fitted = fitKnowledgeFolders([
      folder("folder:1", { x: 0, y: 0 }, { w: 80, h: 80 }),
      folder("folder:2", { x: KNOWLEDGE_FOLDER_PAD, y: KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD }, { w: 80, h: 80 }, "folder:1"),
      card(
        "a",
        { x: KNOWLEDGE_FOLDER_PAD, y: KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD },
        "folder:2",
        { w: 180, h: 60 },
      ),
    ]);
    const inner = fitted.find((node) => node.id === "folder:2")!;
    const outer = fitted.find((node) => node.id === "folder:1")!;
    expect(inner.width).toBe(180 + KNOWLEDGE_FOLDER_PAD * 2);
    expect(outer.width).toBe((inner.width ?? 0) + KNOWLEDGE_FOLDER_PAD * 2);
    expect(outer.height).toBe((inner.height ?? 0) + KNOWLEDGE_FOLDER_HEADER + KNOWLEDGE_FOLDER_PAD * 2);
  });

  it("leaves an already fitted tree unchanged", () => {
    const once = fitKnowledgeFolders([
      folder("folder:1", { x: 10, y: 10 }, { w: 200, h: 160 }),
      card("a", { x: 40, y: 70 }, "folder:1"),
    ]);
    const twice = fitKnowledgeFolders(once);
    expect(twice).toBe(once);
  });
});
