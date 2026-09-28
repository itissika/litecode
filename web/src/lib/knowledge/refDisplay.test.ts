import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import { bodyMarkerKeys, chipForMarker, relationStripChips } from "./refDisplay";
import { normalizeKey } from "./markers";
import type { KnowledgeNode } from "./types";

function node(key: string) {
  return knowledgeFixture.find((item) => item.key === key)!;
}

function index() {
  const byId = new Map(knowledgeFixture.map((n) => [n.id, n]));
  const byKey = new Map(
    knowledgeFixture.map((n) => [normalizeKey(n.key), n]),
  );
  return { byId, byKey };
}

describe("refDisplay", () => {
  it("puts prose faults on markers only", () => {
    const { byKey } = index();
    const broken = node("broken-marker");
    const chip = chipForMarker(broken, "not-a-node", byKey);
    expect(chip.tone).toBe("error");
    expect(chip.jumpable).toBe(false);
  });

  it("shows a relation that the body never cites", () => {
    const source: KnowledgeNode = {
      id: "draft",
      key: "draft",
      value: "plain",
      relations: ["knowledge"],
      status: "enabled",
      path: "draft.md",
      summary: "",
      x: null,
      y: null,
      w: null,
      h: null,
    };
    const target = node("knowledge");
    const byId = new Map<string, KnowledgeNode>([
      [source.id, source],
      [target.id, target],
    ]);
    const strip = relationStripChips(source, byId, bodyMarkerKeys(source.value));
    expect(strip.map((chip) => chip.key)).toEqual(["knowledge"]);
    expect(strip[0]?.tone).toBe("warning");
  });

  it("keeps inactive body citations out of the relation strip", () => {
    const { byId, byKey } = index();
    const sampling = node("sampling");
    const keys = bodyMarkerKeys(sampling.value);
    expect(relationStripChips(sampling, byId, keys)).toEqual([]);
    const chip = chipForMarker(sampling, "temperature", byKey);
    expect(chip.tone).toBe("disabled");
    expect(chip.jumpable).toBe(true);
  });
});
