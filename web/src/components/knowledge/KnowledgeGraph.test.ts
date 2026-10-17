import { describe, expect, it } from "vitest";

import {
  KNOWLEDGE_FOCUS_SCRIM_Z,
  knowledgeCardFlowLayout,
  knowledgeCardZIndex,
} from "./KnowledgeGraph";
import {
  KNOWLEDGE_NODE_HEIGHT,
  KNOWLEDGE_NODE_MAX_HEIGHT,
  KNOWLEDGE_NODE_MAX_WIDTH,
  KNOWLEDGE_NODE_OPEN_HEIGHT,
  KNOWLEDGE_NODE_WIDTH,
} from "../../lib/knowledge/layoutGraph";

describe("knowledgeCardFlowLayout", () => {
  it("uses the fixed collapsed size even when markdown stores expanded dimensions", () => {
    const layout = knowledgeCardFlowLayout(false, { w: 400, h: 320 });
    expect(layout.width).toBe(KNOWLEDGE_NODE_WIDTH);
    expect(layout.height).toBe(KNOWLEDGE_NODE_HEIGHT);
    expect(layout.style.width).toBe(KNOWLEDGE_NODE_WIDTH);
    expect(layout.style.height).toBe(KNOWLEDGE_NODE_HEIGHT);
  });

  it("applies saved dimensions only while expanded", () => {
    const layout = knowledgeCardFlowLayout(true, { w: 400, h: 320 });
    expect(layout.width).toBe(400);
    expect(layout.height).toBe(320);
    expect(layout.style.width).toBe(400);
    expect(layout.style.height).toBe(320);
  });

  it("gives an expanded card a bounded default size instead of growing with content", () => {
    const layout = knowledgeCardFlowLayout(true, undefined);
    expect(layout.width).toBe(KNOWLEDGE_NODE_WIDTH);
    expect(layout.height).toBe(KNOWLEDGE_NODE_OPEN_HEIGHT);
    expect(layout.style.height).toBe(KNOWLEDGE_NODE_OPEN_HEIGHT);
  });

  it("clamps an expanded card to the max width and height", () => {
    const layout = knowledgeCardFlowLayout(true, { w: 900, h: 900 });
    expect(layout.width).toBe(KNOWLEDGE_NODE_MAX_WIDTH);
    expect(layout.height).toBe(KNOWLEDGE_NODE_MAX_HEIGHT);
  });
});

describe("knowledgeCardZIndex", () => {
  it("keeps a plain collapsed card at the base rung", () => {
    expect(knowledgeCardZIndex(false, false, false, false)).toBe(10);
  });

  it("lifts an expanded card above a collapsed non-focused card", () => {
    expect(knowledgeCardZIndex(true, false, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(false, false, false, false),
    );
  });

  it("lifts the focused card above the canvas focus scrim and expanded neighbours", () => {
    expect(knowledgeCardZIndex(false, true, false, false)).toBeGreaterThan(
      KNOWLEDGE_FOCUS_SCRIM_Z,
    );
    expect(knowledgeCardZIndex(false, true, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(true, false, false, false),
    );
  });

  it("lifts citation-linked cards above the focus scrim", () => {
    expect(knowledgeCardZIndex(false, false, true, false)).toBeGreaterThan(
      KNOWLEDGE_FOCUS_SCRIM_Z,
    );
    expect(knowledgeCardZIndex(false, true, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(false, false, true, false),
    );
  });

  it("puts the focused expanded card on top of the other cards", () => {
    expect(knowledgeCardZIndex(true, true, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(true, false, false, false),
    );
    expect(knowledgeCardZIndex(true, true, false, false)).toBeGreaterThan(
      knowledgeCardZIndex(false, true, false, false),
    );
  });

  it("still ranks the card being dragged above the focused expanded card", () => {
    expect(knowledgeCardZIndex(true, true, false, true)).toBeGreaterThan(
      knowledgeCardZIndex(true, true, false, false),
    );
  });
});
