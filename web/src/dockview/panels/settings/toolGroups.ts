import type { AvailableKind, AvailableTool } from "../../../api/settings";

/** One section of Agents → Tool bindings. */
export interface ToolGroup {
  kind: AvailableKind;
  tools: AvailableTool[];
}

/**
 * Section order: what the agent reaches for first (core), then what an engine
 * adds (search / LSP / browser), then workspace-defined tools (custom, MCP).
 */
const GROUP_ORDER: readonly AvailableKind[] = ["core", "engine", "custom", "mcp"];

/**
 * Within a section, code-surface cards come before direct-only ones: the code
 * cards are what a model reaches through `run_code`, so they read first.
 * Cards without a declared surface count as direct.
 */
function surfaceRank(tool: AvailableTool): number {
  return tool.surface === "code" ? 0 : 1;
}

/**
 * Reading order inside the core section: file access → shell → web → session
 * and workspace context → planning → delegation. Ids absent here sort
 * alphabetically after the listed ones, so a new tool lands in a predictable
 * place without editing this table.
 */
const CORE_ORDER: readonly string[] = [
  "read",
  "glob",
  "grep",
  "edit",
  "write",
  "run",
  "bash",
  "webfetch",
  "websearch",
  "session_search",
  "knowledge",
  "litecode_workspace",
  "plan",
  "todo",
  "ask_user",
  "subagent",
  "subagent_tools",
];

function coreRank(id: string): number {
  const at = CORE_ORDER.indexOf(id);
  return at === -1 ? CORE_ORDER.length : at;
}

/** Custom suites carry a human label; every other card falls back to its id. */
function cardLabel(tool: AvailableTool): string {
  return tool.label?.trim() || tool.id;
}

function compareCards(left: AvailableTool, right: AvailableTool): number {
  const bySurface = surfaceRank(left) - surfaceRank(right);
  if (bySurface !== 0) return bySurface;
  // Only core has a curated order; other sections read alphabetically.
  if (left.kind === "core") {
    const byRank = coreRank(left.id) - coreRank(right.id);
    if (byRank !== 0) return byRank;
  } else {
    const byLabel = cardLabel(left).localeCompare(cardLabel(right));
    if (byLabel !== 0) return byLabel;
  }
  return left.id.localeCompare(right.id);
}

/**
 * Split the bindable cards into sections and order each one: core → engine →
 * custom → mcp, and inside a section code-surface cards before direct ones.
 * Sections keep a fixed order regardless of bindings, so cards never jump
 * between groups when a toggle flips — only their own on/off state changes.
 */
export function groupToolCards(tools: readonly AvailableTool[]): ToolGroup[] {
  return GROUP_ORDER.map((kind) => ({
    kind,
    tools: tools.filter((tool) => tool.kind === kind).sort(compareCards),
  })).filter((group) => group.tools.length > 0);
}
