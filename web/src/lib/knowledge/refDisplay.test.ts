import { describe, expect, it } from "vitest";

import { knowledgeFixture } from "../../test/fixtures/knowledge";
import { citationRefKey, citationSource, resolveCitation } from "./refDisplay";
import { logicalKnowledgePath } from "./markers";
import type { KnowledgeRefState, KnowledgeRefStatus } from "./types";

function node(key: string) {
  return knowledgeFixture.find((item) => item.key === key)!;
}

describe("refDisplay", () => {
  it("folds a bare-path citation to a valid knowledge node chip", () => {
    const source = node("session");
    const seq = node("seq");
    const hit = resolveCitation(
      { path: logicalKnowledgePath(seq.path) },
      {
        source,
        target: seq,
        externalOpen: false,
      },
    );
    expect(hit.file).toBe(false);
    expect(hit.resolvable).toBe(true);
    expect(hit.targetId).toBe(seq.id);
    expect(hit.label).toBe("seq");
    expect(hit.detail).toBe(logicalKnowledgePath(seq.path));
  });

  it("shows a disabled target node with disabled tone and jumpable", () => {
    const source = node("sampling");
    const temp = node("temperature");
    const chip = resolveCitation(
      { path: logicalKnowledgePath(temp.path) },
      { source, target: temp, externalOpen: false },
    );
    expect(chip.file).toBe(false);
    expect(chip.tone).toBe("disabled");
    expect(chip.resolvable).toBe(true);
    expect(chip.label).toBe("temperature");
  });

  it("marks a backend-reported dangling knowledge path as an unresolvable node error", () => {
    const source = node("broken-marker");
    const missing = resolveCitation(
      { path: "knowledge/not-a-node.md" },
      {
        source,
        target: null,
        ref: refState("path:knowledge/not-a-node.md", "node", "dangling_relation", "error", {
          path: "knowledge/not-a-node.md",
          reason: "missing",
        }),
        externalOpen: false,
      },
    );
    expect(missing.file).toBe(false);
    expect(missing.resolvable).toBe(false);
    expect(missing.tone).toBe("error");
  });

  it("resolves a workspace file, a symbol, and an external path only when a preview exists", () => {
    const source = node("session");
    const present = resolveCitation(
      { path: "src/a.rs", symbol: "Store.save", line: 4 },
      { source, target: null, externalOpen: false },
    );
    expect(present.file).toBe(true);
    expect(present.resolvable).toBe(true);
    expect(present.symbol).toBe(true);
    expect(present.label).toBe("a.rs : save");

    const outside = resolveCitation(
      { path: "C:/outside/a.ts" },
      { source, target: null, externalOpen: false },
    );
    expect(outside.resolvable).toBe(false);

    const preview = resolveCitation(
      { path: "C:/outside/a.ts" },
      { source, target: null, externalOpen: true },
    );
    expect(preview.resolvable).toBe(true);
  });

  it("aligns file and symbol verdicts by ref_key, so a same-named anchor in another file stays clean", () => {
    const source = node("session");
    const broken = refState("sym:src/a.rs\0new", "symbol", "ambiguous_symbol", "error", {
      path: "src/a.rs",
      symbol: "new",
    });
    const inA = { path: "src/a.rs", symbol: "new" };
    const inB = { path: "src/b.rs", symbol: "new" };
    expect(citationRefKey(inA)).toBe("sym:src/a.rs\0new");
    expect(citationRefKey(inB)).toBe("sym:src/b.rs\0new");
    const a = resolveCitation(inA, { source, target: null, ref: broken, externalOpen: false });
    expect(a.tone).toBe("error");
    expect(a.resolvable).toBe(false);
    expect(a.title).toBe('Symbol "new" in "src/a.rs" is not unique.');
    const b = resolveCitation(inB, { source, target: null, ref: null, externalOpen: false });
    expect(b.tone).toBe("normal");
    expect(b.resolvable).toBe(true);
  });

  it("shows no citation error while the snapshot is not ready", () => {
    const source = node("session");
    const missing = refState("path:src/gone.rs", "file", "missing_file", "error", {
      path: "src/gone.rs",
    });
    const checking = resolveCitation(
      { path: "src/gone.rs" },
      { source, target: null, ref: missing, ready: false, externalOpen: false },
    );
    expect(checking.tone).toBe("normal");
    const node404 = resolveCitation(
      { path: "knowledge/gone.md" },
      {
        source,
        target: null,
        ref: refState("path:knowledge/gone.md", "node", "dangling_relation", "error", {
          path: "knowledge/gone.md",
        }),
        ready: false,
        externalOpen: false,
      },
    );
    expect(node404.tone).not.toBe("error");
  });

  it("uses syntax-only ref_keys and ignores lines on a symbol", () => {
    expect(citationRefKey({ path: "src\\a.rs", lines: "7-7" })).toBe("range:src/a.rs\x007");
    expect(citationRefKey({ path: "a.rs", badLines: "+3" })).toBe("range:a.rs\x00+3");
    expect(citationRefKey({ path: "a.rs", symbol: " Store::save ", lines: "4" })).toBe(
      "sym:a.rs\x00Store.save",
    );
  });

  it("treats an unknown future error code as an error by severity", () => {
    const source = node("session");
    const future = refState(
      "sym:a.rs\0save",
      "symbol",
      "future_error" as KnowledgeRefStatus,
      "error",
      {},
    );
    const chip = resolveCitation(
      { path: "a.rs", symbol: "save" },
      { source, target: null, ref: { ...future, message: "future detail" }, externalOpen: false },
    );
    expect(chip.tone).toBe("error");
    expect(chip.title).toBe("future detail");
  });

  it("writes a citation back as a standard CommonMark Markdown link", () => {
    const seq = node("seq");
    expect(citationSource({ path: logicalKnowledgePath(seq.path) }, seq)).toBe(
      `[seq](${logicalKnowledgePath(seq.path)})`,
    );
    expect(citationSource({ path: "src/a.rs" })).toBe("[a.rs](src/a.rs)");
    expect(citationSource({ path: "src/a.rs", lines: "4-9" })).toBe(
      "[a.rs:4-9](src/a.rs#L4-L9)",
    );
    expect(citationSource({ path: "src/a.rs", line: 42 })).toBe(
      "[a.rs:42](src/a.rs#L42)",
    );
    expect(
      citationSource({
        path: "src/a.rs",
        symbol: "Store.save",
        lines: "4-9",
      }),
    ).toBe("[Store.save](src/a.rs#Store.save)");
  });
});

function refState(
  refKey: string,
  kind: KnowledgeRefState["kind"],
  status: KnowledgeRefStatus,
  severity: "error" | "warning" | null,
  args: Record<string, unknown>,
): KnowledgeRefState {
  return {
    refKey,
    kind,
    key: typeof args.key === "string" ? args.key : null,
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
