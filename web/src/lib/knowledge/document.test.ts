import { describe, expect, it } from "vitest";

import {
  knowledgeFromFiles,
  parseKnowledgeMarkdown,
  renderKnowledgeMarkdown,
} from "./document";
import { extractMarkers } from "./markers";

describe("parseKnowledgeMarkdown", () => {
  it("reads the leading node fence and leaves the body", () => {
    const markdown = renderKnowledgeMarkdown({
      key: "seq",
      status: "disabled",
      body: "以 [[node : session]] 递增。",
    });
    const parsed = parseKnowledgeMarkdown("内核/seq.md", markdown);
    expect(parsed.key).toBe("seq");
    expect(parsed.status).toBe("disabled");
    expect(parsed.folderId).toBe("内核");
    expect(parsed.body).toBe("以 [[node : session]] 递增。\n");
    expect(extractMarkers(parsed.body)).toEqual(["session"]);
  });

  it("accepts a tight colon and ignores a later node fence in the body", () => {
    const markdown = [
      "```node",
      "node:seq",
      "```",
      "",
      "正文",
      "```node",
      "node : other",
      "```",
    ].join("\n");
    const parsed = parseKnowledgeMarkdown("seq.md", markdown);
    expect(parsed.key).toBe("seq");
    expect(parsed.body).toContain("node : other");
    expect(extractMarkers(parsed.body)).toEqual([]);
  });

  it("does not treat prose before the fence as a declaration", () => {
    const markdown = "先写一句。\n```node\nnode : seq\n```\n";
    const parsed = parseKnowledgeMarkdown("seq.md", markdown);
    expect(parsed.key).toBe("");
    expect(parsed.body.startsWith("先写一句。")).toBe(true);
  });
});

describe("knowledgeFromFiles", () => {
  it("uses a unique key as the id and derives relations from the body", () => {
    const { nodes, folders } = knowledgeFromFiles([
      {
        path: "内核/session.md",
        markdown: renderKnowledgeMarkdown({
          key: "session",
          status: "enabled",
          body: "见 [[node:seq]] 与 [[node : seq]]。",
          refs: ["seq"],
        }),
      },
      {
        path: "内核/上下文/seq.md",
        markdown: renderKnowledgeMarkdown({
          key: "seq",
          status: "enabled",
          body: "序号。",
        }),
      },
    ]);
    const session = nodes.find((node) => node.key === "session");
    expect(session?.id).toBe("session");
    expect(session?.relations).toEqual(["seq"]);
    expect(session?.folderId).toBe("内核");
    expect(folders.map((folder) => folder.id)).toEqual(["内核", "内核/上下文"]);
    expect(folders.find((folder) => folder.id === "内核/上下文")?.parentId).toBe(
      "内核",
    );
  });

  it("keeps both copies when a key is declared twice", () => {
    const { nodes } = knowledgeFromFiles([
      {
        path: "a.md",
        markdown: renderKnowledgeMarkdown({
          key: "seq",
          status: "enabled",
          body: "一",
        }),
      },
      {
        path: "b.md",
        markdown: renderKnowledgeMarkdown({
          key: "seq",
          status: "enabled",
          body: "二",
        }),
      },
    ]);
    expect(new Set(nodes.map((node) => node.id)).size).toBe(2);
  });
});
