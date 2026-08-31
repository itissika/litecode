import { describe, expect, it } from "vitest";

import {
  documentIsFresh,
  isWorkspaceExcludesPath,
  mergeLayeredMcp,
  SECTION_DOCUMENTS,
  sectionNeedsSkeleton,
  splitMcpListing,
  type SettingsDataProbe,
} from "./settingsDocuments";

function emptyProbe(patch: Partial<SettingsDataProbe> = {}): SettingsDataProbe {
  return {
    summary: null,
    adapters: [],
    providers: null,
    models: null,
    availableTools: null,
    customTools: null,
    mcpDefs: null,
    mcpRuntime: null,
    agents: {},
    log: null,
    websearch: null,
    excludes: null,
    docClock: {},
    ...patch,
  };
}

describe("SECTION_DOCUMENTS", () => {
  it("does not attach engines or excludes to Provider", () => {
    expect(SECTION_DOCUMENTS.connection).toEqual(["summary", "adapters", "providers"]);
    expect(SECTION_DOCUMENTS.engines).toEqual([]);
    expect(SECTION_DOCUMENTS.files).toEqual(["excludes"]);
    expect(SECTION_DOCUMENTS.models).not.toContain("agents");
  });
});

describe("sectionNeedsSkeleton", () => {
  it("treats Provider as ready once providers are present", () => {
    expect(sectionNeedsSkeleton("connection", emptyProbe())).toBe(true);
    expect(
      sectionNeedsSkeleton("connection", emptyProbe({ providers: {} })),
    ).toBe(false);
  });

  it("does not skeleton Engines", () => {
    expect(sectionNeedsSkeleton("engines", emptyProbe())).toBe(false);
  });
});

describe("documentIsFresh", () => {
  it("compares revisioned docs to settings revision and ignores it for excludes", () => {
    expect(
      documentIsFresh("providers", { revision: 2, docClock: { providers: 1 } }),
    ).toBe(false);
    expect(
      documentIsFresh("providers", { revision: 2, docClock: { providers: 2 } }),
    ).toBe(true);
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
