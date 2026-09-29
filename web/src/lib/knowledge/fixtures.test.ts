import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { knowledgeFromFiles, renderKnowledgeMarkdown } from "./document";
import { validateKnowledge } from "./validate";

const fixtureRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../../tests/fixtures/knowledge",
);

interface ExpectedFile {
  path: string;
  key: string;
  status: string;
  invalidStatus: string | null;
  summary: string;
  refs: string[];
  extras: string[];
}

interface ExpectedIssue {
  node: string;
  code: string;
}

function readTree(dir: string, prefix = ""): { path: string; markdown: string }[] {
  const out: { path: string; markdown: string }[] = [];
  for (const name of fs.readdirSync(dir)) {
    if (name.startsWith(".")) continue;
    const abs = path.join(dir, name);
    const rel = prefix ? `${prefix}/${name}` : name;
    if (fs.statSync(abs).isDirectory()) {
      out.push(...readTree(abs, rel));
      continue;
    }
    if (!name.toLowerCase().endsWith(".md")) continue;
    out.push({ path: rel.replaceAll("\\", "/"), markdown: fs.readFileSync(abs, "utf8") });
  }
  out.sort((a, b) => a.path.localeCompare(b.path));
  return out;
}

describe("shared knowledge fixtures", () => {
  it("parses the same keys, refs, extras, and issue codes", () => {
    const expected = JSON.parse(
      fs.readFileSync(path.join(fixtureRoot, "expected.json"), "utf8"),
    ) as { files: ExpectedFile[]; issues: ExpectedIssue[] };
    const files = readTree(fixtureRoot);
    const { nodes } = knowledgeFromFiles(files);
    const actual = nodes
      .map((node) => ({
        path: node.path,
        key: node.key,
        status: node.status,
        invalidStatus: node.invalidStatus ?? null,
        summary: node.summary,
        refs: node.relations,
        extras: node.extras ?? [],
      }))
      .sort((a, b) => a.path.localeCompare(b.path));
    const wanted = [...expected.files].sort((a, b) => a.path.localeCompare(b.path));
    expect(actual).toEqual(wanted);

    const issues = validateKnowledge(nodes)
      .map((issue) => ({ node: issue.nodeId, code: issue.code }))
      .sort((a, b) => `${a.node}:${a.code}`.localeCompare(`${b.node}:${b.code}`));
    const wantedIssues = [...expected.issues].sort((a, b) =>
      `${a.node}:${a.code}`.localeCompare(`${b.node}:${b.code}`),
    );
    expect(issues).toEqual(wantedIssues);
  });

  it("keeps an unknown declaration line across a save", () => {
    const source = fs.readFileSync(path.join(fixtureRoot, "内核", "seq.md"), "utf8");
    const { nodes } = knowledgeFromFiles([{ path: "内核/seq.md", markdown: source }]);
    const node = nodes[0];
    const saved = renderKnowledgeMarkdown({
      key: node.key,
      status: node.status,
      summary: node.summary,
      body: node.value,
      x: node.x,
      y: node.y,
      w: node.w,
      h: node.h,
      extras: node.extras,
    });
    const again = knowledgeFromFiles([{ path: "内核/seq.md", markdown: saved }]).nodes[0];
    expect(again.extras).toEqual(["tags : a"]);
  });

  it("writes an illegal status back unchanged", () => {
    const source = fs.readFileSync(path.join(fixtureRoot, "bad-status.md"), "utf8");
    const node = knowledgeFromFiles([{ path: "bad-status.md", markdown: source }]).nodes[0]!;
    expect(node.status).toBe("enabled");
    expect(node.invalidStatus).toBe("nope");
    const saved = renderKnowledgeMarkdown({
      key: node.key,
      status: node.status,
      invalidStatus: node.invalidStatus,
      summary: node.summary,
      body: node.value,
      extras: node.extras,
    });
    expect(saved).toContain("status : nope");
    const again = knowledgeFromFiles([{ path: "bad-status.md", markdown: saved }]).nodes[0]!;
    expect(again.invalidStatus).toBe("nope");
  });
});
