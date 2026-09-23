import { describe, expect, it } from "vitest";

import type { LlmSettings } from "../api/settings";
import {
  documentIsFresh,
  isWorkspaceCustomToolsPath,
  isWorkspaceExcludesPath,
  isWorkspaceMcpPath,
  mergeLayeredMcp,
  SECTION_DOCUMENTS,
  sectionNeedsSkeleton,
  settingsDocsForEvent,
  splitMcpListing,
  type SettingsDataProbe,
} from "./settingsDocuments";

const llmDoc: LlmSettings = {
  catalog_path: "C:\\x\\provider-catalog.toml",
  revision: 1,
  providers: [],
  active_models: [],
};

function emptyProbe(patch: Partial<SettingsDataProbe> = {}): SettingsDataProbe {
  return {
    summary: null,
    llm: null,
    availableTools: null,
    customTools: null,
    mcpDefs: null,
    mcpRuntime: null,
    agents: {},
    log: null,
    websearch: null,
    excludes: null,
    engines: null,
    docClock: {},
    ...patch,
  };
}

describe("SECTION_DOCUMENTS", () => {
  it("has exactly one LLM document and never asks for adapters/providers/models", () => {
    expect(SECTION_DOCUMENTS.connection).toEqual(["summary", "llm"]);
    expect(SECTION_DOCUMENTS.agents).toContain("llm");
    expect(SECTION_DOCUMENTS.engines).toEqual(["engines"]);
    expect(SECTION_DOCUMENTS.files).toEqual(["excludes"]);
    const allDocs = Object.values(SECTION_DOCUMENTS).flat();
    expect(allDocs).not.toContain("adapters");
    expect(allDocs).not.toContain("providers");
    expect(allDocs).not.toContain("models");
  });
});

describe("settingsDocsForEvent", () => {
  it("maps the single llm document id", () => {
    expect(settingsDocsForEvent(["llm"])).toEqual(["llm"]);
  });

  it("ignores the deleted providers/models document ids", () => {
    expect(settingsDocsForEvent(["providers"])).toEqual([]);
    expect(settingsDocsForEvent(["models"])).toEqual([]);
    expect(settingsDocsForEvent(["models", "llm"])).toEqual(["llm"]);
  });
});

describe("sectionNeedsSkeleton", () => {
  it("treats Provider as ready once the llm document is present", () => {
    expect(sectionNeedsSkeleton("connection", emptyProbe())).toBe(true);
    expect(sectionNeedsSkeleton("connection", emptyProbe({ llm: llmDoc }))).toBe(
      false,
    );
  });

  it("skeletons Agents until llm + agents are loaded", () => {
    expect(
      sectionNeedsSkeleton("agents", emptyProbe({ agents: {} })),
    ).toBe(true);
    expect(
      sectionNeedsSkeleton(
        "agents",
        emptyProbe({
          agents: {},
          docClock: { agents: 1 },
          llm: llmDoc,
          availableTools: [],
          mcpDefs: { global: [], workspace: [] },
          mcpRuntime: { global: {}, workspace: {} },
        }),
      ),
    ).toBe(false);
  });

  it("skeletons Engines until the engines document is loaded", () => {
    expect(sectionNeedsSkeleton("engines", emptyProbe())).toBe(true);
    expect(
      sectionNeedsSkeleton(
        "engines",
        emptyProbe({
          engines: {
            version: 1,
            lsp: { desired: false, servers: [] },
            retrieval: { desired: false },
          },
        }),
      ),
    ).toBe(false);
  });
});

describe("documentIsFresh", () => {
  it("compares revisioned docs to settings revision and ignores it for excludes", () => {
    expect(documentIsFresh("llm", { revision: 2, docClock: { llm: 1 } })).toBe(
      false,
    );
    expect(documentIsFresh("llm", { revision: 2, docClock: { llm: 2 } })).toBe(
      true,
    );
    expect(
      documentIsFresh("excludes", { revision: 9, docClock: { excludes: 1 } }),
    ).toBe(true);
  });
});

describe("isWorkspaceExcludesPath", () => {
  it("matches the workspace excludes file", () => {
    expect(isWorkspaceExcludesPath(".litecode/excludes.json")).toBe(true);
    expect(isWorkspaceExcludesPath("src/.litecode/excludes.json")).toBe(true);
    expect(isWorkspaceExcludesPath(".litecode/engines.json")).toBe(false);
  });
});

describe("workspace tool def paths", () => {
  it("matches mcp.json and custom_tools.json under .litecode", () => {
    expect(isWorkspaceMcpPath(".litecode/mcp.json")).toBe(true);
    expect(isWorkspaceMcpPath("repo/.litecode/mcp.json")).toBe(true);
    expect(isWorkspaceCustomToolsPath(".litecode/custom_tools.json")).toBe(true);
    expect(isWorkspaceMcpPath(".litecode/excludes.json")).toBe(false);
    expect(isWorkspaceCustomToolsPath(".litecode/mcp.json")).toBe(false);
  });
});

describe("splitMcpListing / mergeLayeredMcp", () => {
  it("splits runtime fields off the definition and merges them back", () => {
    const split = splitMcpListing({
      global: [
        {
          id: "fs",
          command: "npx",
          origin: "global",
          status: "running",
          tools: [{ name: "read", description: "" }],
          error: null,
        },
      ],
      workspace: [],
    });
    expect(split.mcpDefs.global[0]).toMatchObject({ id: "fs", command: "npx" });
    expect(split.mcpDefs.global[0]).not.toHaveProperty("status");
    expect(split.mcpRuntime.global.fs.status).toBe("running");
    const merged = mergeLayeredMcp(split.mcpDefs, split.mcpRuntime);
    expect(merged.global[0].status).toBe("running");
    expect(merged.global[0].tools?.[0].name).toBe("read");
  });
});
