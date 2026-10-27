import { describe, expect, it } from "vitest";

import type { AvailableKind, AvailableTool } from "../../../api/settings";
import { groupToolCards } from "./toolGroups";

function card(
  id: string,
  kind: AvailableKind,
  patch: Partial<AvailableTool> = {},
): AvailableTool {
  return { id, kind, origin: "builtin", permission_surface: "fixed", ...patch };
}

describe("groupToolCards", () => {
  it("keeps a fixed section order and drops empty sections", () => {
    const groups = groupToolCards([
      card("mcp_demo", "mcp"),
      card("read", "core"),
      card("lsp", "engine"),
    ]);
    expect(groups.map((group) => group.kind)).toEqual(["core", "engine", "mcp"]);
    expect(groups.map((group) => group.tools.map((tool) => tool.id))).toEqual([
      ["read"],
      ["lsp"],
      ["mcp_demo"],
    ]);
  });

  it("orders core cards by reading order and puts unknown ids last", () => {
    const [core] = groupToolCards([
      card("write", "core"),
      card("zeta", "core"),
      card("run", "core"),
      card("read", "core"),
      card("alpha", "core"),
      card("edit", "core"),
    ]);
    expect(core.tools.map((tool) => tool.id)).toEqual([
      "read",
      "edit",
      "write",
      "run",
      "alpha",
      "zeta",
    ]);
  });

  it("puts code-surface cards before direct ones in every section", () => {
    const groups = groupToolCards([
      card("run", "core", { surface: undefined }),
      card("plan", "core", { surface: "direct" }),
      card("write", "core", { surface: "code" }),
      card("read", "core", { surface: "code" }),
      card("lsp", "engine", { surface: "direct" }),
      card("code_search", "engine", { surface: "code" }),
    ]);
    expect(groups.map((group) => group.tools.map((tool) => tool.id))).toEqual([
      ["read", "write", "run", "plan"],
      ["code_search", "lsp"],
    ]);
  });

  it("orders other sections by label, then id", () => {
    const groups = groupToolCards([
      card("suite:zzz", "custom", { label: "alpha" }),
      card("suite:aaa", "custom", { label: "beta" }),
      card("suite:mm", "custom", { label: "  " }),
    ]);
    expect(groups[0].tools.map((tool) => tool.id)).toEqual([
      "suite:zzz",
      "suite:aaa",
      "suite:mm",
    ]);
  });

  it("leaves the caller's array untouched", () => {
    const tools = [card("write", "core"), card("read", "core")];
    groupToolCards(tools);
    expect(tools.map((tool) => tool.id)).toEqual(["write", "read"]);
  });
});
