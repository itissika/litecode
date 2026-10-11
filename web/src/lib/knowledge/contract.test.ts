import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { knowledgeFromFiles } from "./document";
import { isKnowledgeIssueCode } from "./issueText";
import { extractRefs, knowledgeRefKey } from "./markers";
import { refStatusTone } from "./refDisplay";
import { knowledgeNodeIds } from "./snapshot";

/** Shared backend/frontend contract: `tests/fixtures/knowledge_refs/cases.json`. */
const casesPath = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../../tests/fixtures/knowledge_refs/cases.json",
);

interface ExpectedRef {
  kind: string;
  key?: string;
  path?: string;
  symbol?: string;
  lines?: string | null;
  bad_lines?: string;
  ref_key: string;
}

interface Cases {
  version: number;
  extract: { id: string; text: string; refs: ExpectedRef[] }[];
  resolve: { id: string; expect: { status: string }[] }[];
  layout: { id: string; knowledge: Record<string, string>; nodes: string[] }[];
}

const cases = JSON.parse(fs.readFileSync(casesPath, "utf8")) as Cases;

/**
 * What the frontend must agree on. A symbol never carries `lines`; any it was
 * written with show up as `bad_lines` (KR-SYM-6), and its ref_key never
 * includes them.
 */
function shape(ref: ExpectedRef) {
  return {
    kind: ref.kind,
    key: ref.key,
    path: ref.path,
    symbol: ref.symbol,
    lines: ref.kind === "range" || ref.kind === "symbol" ? (ref.lines ?? null) : undefined,
    bad_lines: ref.bad_lines,
    ref_key: ref.ref_key,
  };
}

describe("knowledge_refs cases.json", () => {
  it("is the v2 contract", () => {
    expect(cases.version).toBe(2);
  });

  for (const item of cases.extract) {
    it(`extract ${item.id}`, () => {
      const got = extractRefs(item.text);
      expect(got.map(shape)).toEqual(item.refs.map(shape));
      for (const ref of got) {
        expect(
          knowledgeRefKey(
            ref.kind === "node"
              ? { kind: "node", key: ref.key ?? "" }
              : {
                  kind: "file",
                  path: ref.path ?? "",
                  symbol: ref.symbol ?? null,
                  lines: ref.kind === "range" ? (ref.lines ?? ref.bad_lines ?? null) : null,
                },
          ),
        ).toBe(ref.ref_key);
        if (ref.kind === "symbol") {
          expect(ref.lines ?? null).toBeNull();
          expect(ref.ref_key).not.toMatch(/\0.*\0/);
        }
      }
    });
  }

  it("knows every status the backend can report", () => {
    const statuses = new Set(
      cases.resolve.flatMap((item) => item.expect.map((row) => row.status)),
    );
    for (const status of statuses) {
      expect(isKnowledgeIssueCode(status), status).toBe(true);
      if (!isKnowledgeIssueCode(status)) continue;
      // A problem never reads as plain ok (inactive_target styles via the target node).
      if (status !== "inactive_target") {
        expect(refStatusTone({ status, severity: null }), status).not.toBe("ok");
      }
    }
  });

  it("derives node ids exactly like knowledgeFromFiles", () => {
    for (const item of cases.layout) {
      const files = Object.entries(item.knowledge)
        .filter(([rel]) => rel.endsWith(".md"))
        .map(([rel, markdown]) => ({ path: rel, markdown }));
      const local = knowledgeFromFiles(files).nodes;
      expect(local.map((node) => node.path).sort()).toEqual([...item.nodes].sort());
      expect(knowledgeNodeIds(local)).toEqual(local.map((node) => node.id));
    }
  });
});
