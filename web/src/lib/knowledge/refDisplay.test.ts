import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "../../test/fixtures/knowledge";
import { chipForMarker, citationRefKey, relationStripChips, resolveCitation } from "./refDisplay";
import { normalizeKey } from "./markers";
import type { KnowledgeNode, KnowledgeRefState, KnowledgeRefStatus } from "./types";

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
        externalOpen: false,
      },
    );
    expect(hit.resolvable).toBe(true);
    expect(hit.targetId).toBe(byKey.get("seq")?.id);

    const missing = resolveCitation(
      { kind: "node", key: "not-a-node" },
      { source, target: null, externalOpen: false },
    );
    expect(missing.resolvable).toBe(false);
    expect(missing.tone).toBe("error");
  });

  it("resolves a workspace file, a drifted symbol, and an external path only when a preview exists", () => {
    const source = node("session");
    const present = resolveCitation(
      { kind: "file", path: "src/a.rs", symbol: "fn save", line: 4 },
      { source, target: null, externalOpen: false },
    );
    expect(present.resolvable).toBe(true);
    expect(present.symbol).toBe(true);
    expect(present.label).toBe("a.rs : fn save");

    const drifted = resolveCitation(
      { kind: "file", path: "src/a.rs", symbol: "fn save" },
      {
        source,
        target: null,
        ref: refState("sym:src/a.rs\0fn save", "symbol_drift", "warning", {
          path: "src/a.rs",
          symbol: "fn save",
        }),
        externalOpen: false,
      },
    );
    expect(drifted.resolvable).toBe(true);
    expect(drifted.tone).toBe("drift");

    const outside = resolveCitation(
      { kind: "file", path: "C:/outside/a.ts" },
      { source, target: null, externalOpen: false },
    );
    expect(outside.resolvable).toBe(false);

    const preview = resolveCitation(
      { kind: "file", path: "C:/outside/a.ts" },
      { source, target: null, externalOpen: true },
    );
    expect(preview.resolvable).toBe(true);
  });

  it("aligns file and symbol verdicts by ref_key, so a same-named chain in another file stays clean", () => {
    const source = node("session");
    const broken = refState("sym:src/a.rs\0fn new", "ambiguous_symbol", "error", {
      path: "src/a.rs",
      symbol: "fn new",
    });
    const inA = { kind: "file" as const, path: "src/a.rs", symbol: "fn new" };
    const inB = { kind: "file" as const, path: "src/b.rs", symbol: "fn new" };
    expect(citationRefKey(inA)).toBe("sym:src/a.rs\0fn new");
    expect(citationRefKey(inB)).toBe("sym:src/b.rs\0fn new");
    const a = resolveCitation(inA, { source, target: null, ref: broken, externalOpen: false });
    expect(a.tone).toBe("error");
    expect(a.resolvable).toBe(false);
    expect(a.title).toBe('Symbol "fn new" in "src/a.rs" is not unique.');
    // b.rs has no verdict of its own yet: neutral "checking", not a's error.
    const b = resolveCitation(inB, { source, target: null, ref: null, externalOpen: false });
    expect(b.tone).toBe("normal");
    expect(b.resolvable).toBe(true);
  });

  it("shows no citation error while the snapshot is not ready", () => {
    const source = node("session");
    const missing = refState("file:src/gone.rs", "missing_file", "error", { path: "src/gone.rs" });
    const checking = resolveCitation(
      { kind: "file", path: "src/gone.rs" },
      { source, target: null, ref: missing, ready: false, externalOpen: false },
    );
    expect(checking.tone).toBe("normal");
    const node404 = resolveCitation(
      { kind: "node", key: "gone" },
      { source, target: null, ref: null, ready: false, externalOpen: false },
    );
    expect(node404.tone).not.toBe("error");
  });

  it("derives a node citation locally until the backend has it, then the backend wins", () => {
    const source = node("session");
    const local = resolveCitation(
      { kind: "node", key: "gone" },
      { source, target: null, ref: null, externalOpen: false },
    );
    expect(local.tone).toBe("error");
    const held = resolveCitation(
      { kind: "node", key: "gone" },
      {
        source,
        target: null,
        ref: refState("node:gone", "ok", null, { key: "gone" }),
        externalOpen: false,
      },
    );
    expect(held.tone).toBe("normal");
  });

  it("uses the range key for a line range and ignores lines on a symbol", () => {
    expect(citationRefKey({ kind: "file", path: "src\\a.rs", lines: "7-7" })).toBe(
      "range:src/a.rs\x007",
    );
    expect(citationRefKey({ kind: "file", path: "a.rs", lines: "+3" })).toBe("range:a.rs\x00+3");
    expect(
      citationRefKey({ kind: "file", path: "a.rs", symbol: " fn a ", lines: "4" }),
    ).toBe("sym:a.rs\x00fn a");
  });

  it("treats an unknown future error code as an error by severity", () => {
    const source = node("session");
    const future = refState("sym:a.rs\0fn a", "symbol_with_lines" as KnowledgeRefStatus, "error", {});
    const chip = resolveCitation(
      { kind: "file", path: "a.rs", symbol: "fn a" },
      { source, target: null, ref: { ...future, message: "lines on a symbol" }, externalOpen: false },
    );
    expect(chip.tone).toBe("error");
    expect(chip.title).toBe("lines on a symbol");
  });
});

function refState(
  refKey: string,
  status: KnowledgeRefStatus,
  severity: "error" | "warning" | null,
  args: Record<string, unknown>,
): KnowledgeRefState {
  const [kind] = refKey.split(":");
  return {
    refKey,
    kind: kind === "sym" ? "symbol" : (kind as KnowledgeRefState["kind"]),
    key: null,
    path: typeof args.path === "string" ? args.path : null,
    symbol: typeof args.symbol === "string" ? args.symbol : null,
    lines: null,
    raw: "",
    status,
    severity,
    targetPath: null,
    args,
    message: "",
  };
}
