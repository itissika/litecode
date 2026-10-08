import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "./fixture";
import { chipForMarker, relationStripChips, resolveCitation } from "./refDisplay";
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
  it("marks a mention whose id does not exist", () => {
    const { byKey } = index();
    const broken = node("broken-marker");
    const chip = chipForMarker(broken, "not-a-node", byKey);
    expect(chip.tone).toBe("error");
    expect(chip.jumpable).toBe(false);
  });

  it("does not invent a chip for a relation the body never cites", () => {
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
    expect(relationStripChips(source, byId)).toEqual([]);
  });

  it("shows an inactive mention with its label", () => {
    const { byId, byKey } = index();
    const sampling = node("sampling");
    const strip = relationStripChips(sampling, byId);
    expect(strip.map((chip) => chip.key)).toEqual(["temperature"]);
    expect(strip[0]?.label).toBe("temperature");
    expect(strip[0]?.tone).toBe("disabled");
    const chip = chipForMarker(sampling, "temperature", byKey);
    expect(chip.tone).toBe("disabled");
    expect(chip.jumpable).toBe(true);
  });

  it("resolves a node citation and leaves a missing key inert", () => {
    const { byKey } = index();
    const source = node("session");
    const hit = resolveCitation(
      { kind: "node", key: "seq" },
      {
        source,
        target: byKey.get("seq") ?? null,
        issues: [],
        externalOpen: false,
      },
    );
    expect(hit.resolvable).toBe(true);
    expect(hit.targetId).toBe(byKey.get("seq")?.id);

    const missing = resolveCitation(
      { kind: "node", key: "not-a-node" },
      { source, target: null, issues: [], externalOpen: false },
    );
    expect(missing.resolvable).toBe(false);
    expect(missing.tone).toBe("error");
  });

  it("resolves a workspace file, a drifted symbol, and an external path only when a preview exists", () => {
    const source = node("session");
    const present = resolveCitation(
      { kind: "file", path: "src/a.rs", symbol: "fn save", line: 4 },
      { source, target: null, issues: [], externalOpen: false },
    );
    expect(present.resolvable).toBe(true);
    expect(present.symbol).toBe(true);
    expect(present.label).toBe("a.rs : fn save");

    const drifted = resolveCitation(
      { kind: "file", path: "src/a.rs", symbol: "fn save" },
      {
        source,
        target: null,
        issues: [
          {
            nodeId: source.id,
            severity: "warning",
            code: "symbol_drift",
            message: "moved",
            ref: "fn save",
          },
        ],
        externalOpen: false,
      },
    );
    expect(drifted.resolvable).toBe(true);
    expect(drifted.tone).toBe("drift");

    const outside = resolveCitation(
      { kind: "file", path: "C:/outside/a.ts" },
      { source, target: null, issues: [], externalOpen: false },
    );
    expect(outside.resolvable).toBe(false);

    const preview = resolveCitation(
      { kind: "file", path: "C:/outside/a.ts" },
      { source, target: null, issues: [], externalOpen: true },
    );
    expect(preview.resolvable).toBe(true);
  });
});
