import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import { bodyMarkerKeys, chipForMarker, relationStripChips } from "./refDisplay";
import { normalizeKey } from "./markers";

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

  it("shows unused registered relations in the strip, not dangling ids", () => {
    const { byId } = index();
    const draft = node("draft-link");
    const keys = bodyMarkerKeys(draft.value);
    const strip = relationStripChips(draft, byId, keys);
    expect(strip.map((c) => c.key)).toEqual(["knowledge"]);
    expect(strip[0]?.tone).toBe("warning");

    const dangling = node("dangling");
    expect(relationStripChips(dangling, byId, bodyMarkerKeys(dangling.value))).toEqual(
      [],
    );
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
