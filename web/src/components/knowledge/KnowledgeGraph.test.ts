import { describe, expect, it } from "vitest";

import { knowledgeCardZIndex } from "./KnowledgeGraph";

describe("knowledgeCardZIndex", () => {
  it("keeps a plain collapsed card at the base rung", () => {
    expect(knowledgeCardZIndex(false, false, false)).toBe(1);
  });

  it("lifts an expanded card above every collapsed card", () => {
    expect(knowledgeCardZIndex(true, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(false, true, false),
    );
    expect(knowledgeCardZIndex(true, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(false, false, false),
    );
  });

  it("puts the focused expanded card on top of the other cards", () => {
    expect(knowledgeCardZIndex(true, true, false)).toBeGreaterThan(
      knowledgeCardZIndex(true, false, false),
    );
    expect(knowledgeCardZIndex(true, true, false)).toBeGreaterThan(
      knowledgeCardZIndex(false, true, false),
    );
  });

  it("still ranks the card being dragged above the focused expanded card", () => {
    expect(knowledgeCardZIndex(true, true, true)).toBeGreaterThan(
      knowledgeCardZIndex(true, true, false),
    );
  });
});
