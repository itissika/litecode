import type {
  AdapterDescriptor,
  AgentProfile,
  AvailableTool,
  CustomToolDefinition,
  LayeredList,
  LogSettings,
  McpServerDefinition,
  McpServerItem,
  McpRunState,
  McpToolInfo,
  ModelDefinition,
  ProviderView,
  SettingsSummary,
  ToolOrigin,
  WebSearchView,
  WorkspaceExcludes,
} from "../api/settings";

export type SettingsSection =
  | "connection"
  | "models"
  | "engines"
  | "custom-tools"
  | "mcp"
  | "agents"
  | "files"
  | "advanced";

/** Fetchable settings documents. `mcp` is one GET split into defs + runtime. */
export type SettingsDocument =
  | "summary"
  | "adapters"
  | "providers"
  | "models"
  | "agents"
  | "availableTools"
  | "customTools"
  | "mcp"
  | "log"
  | "websearch"
  | "excludes";

export const SECTION_DOCUMENTS: Record<SettingsSection, readonly SettingsDocument[]> = {
  connection: ["summary", "adapters", "providers"],
  models: ["adapters", "providers", "models"],
  agents: ["models", "availableTools", "agents", "mcp"],
  "custom-tools": ["customTools"],
  mcp: ["mcp"],
  files: ["excludes"],
  advanced: ["log", "websearch"],
  engines: [],
};

export type McpDefItem = McpServerDefinition & {
  id: string;
  origin?: ToolOrigin;
};

export type McpRuntimeItem = {
  status?: McpRunState;
  tools?: McpToolInfo[];
  error?: string | null;
};

export type LayeredMcpRuntime = {
  global: Record<string, McpRuntimeItem>;
  workspace: Record<string, McpRuntimeItem>;
};

export type SettingsDocClock = Partial<Record<SettingsDocument, number>>;

export const EXCLUDES_CLOCK = 1;

export interface SettingsDataProbe {
  summary: SettingsSummary | null;
  adapters: AdapterDescriptor[];
  providers: Record<string, ProviderView> | null;
  models: Record<string, ModelDefinition> | null;
  availableTools: AvailableTool[] | null;
  customTools: LayeredList<CustomToolDefinition> | null;
  mcpDefs: LayeredList<McpDefItem> | null;
  mcpRuntime: LayeredMcpRuntime | null;
  agents: Record<string, AgentProfile>;
  log: LogSettings | null;
  websearch: WebSearchView | null;
  excludes: WorkspaceExcludes | null;
  docClock: SettingsDocClock;
}

export function documentHasData(doc: SettingsDocument, state: SettingsDataProbe): boolean {
  switch (doc) {
    case "summary":
      return state.summary !== null;
    case "adapters":
      return state.docClock.adapters != null;
    case "providers":
      return state.providers !== null;
    case "models":
      return state.models !== null;
    case "agents":
      return state.docClock.agents != null;
    case "availableTools":
      return state.availableTools !== null;
    case "customTools":
      return state.customTools !== null;
    case "mcp":
      return state.mcpDefs !== null && state.mcpRuntime !== null;
    case "log":
      return state.log !== null;
    case "websearch":
      return state.websearch !== null;
    case "excludes":
      return state.excludes !== null;
  }
}

/** Page body is ready when every render-blocking doc for the tab has data. */
export function sectionNeedsSkeleton(
  section: SettingsSection,
  state: SettingsDataProbe,
): boolean {
  return SECTION_DOCUMENTS[section].some((doc) => {
    if (doc === "summary" || doc === "adapters") return false;
    return !documentHasData(doc, state);
  });
}

export function isRevisionedDocument(doc: SettingsDocument): boolean {
  return doc !== "excludes";
}

export function documentIsFresh(
  doc: SettingsDocument,
  state: { revision: number; docClock: SettingsDocClock },
): boolean {
  const clock = state.docClock[doc];
  if (clock == null) return false;
  if (doc === "excludes") return true;
  return clock === state.revision;
}

export function isWorkspaceExcludesPath(path: string): boolean {
  return isWorkspaceLitecodeJson(path, "excludes.json");
}

export function isWorkspaceMcpPath(path: string): boolean {
  return isWorkspaceLitecodeJson(path, "mcp.json");
}

export function isWorkspaceCustomToolsPath(path: string): boolean {
  return isWorkspaceLitecodeJson(path, "custom_tools.json");
}

function isWorkspaceLitecodeJson(path: string, file: string): boolean {
  const n = path.replace(/\\/g, "/");
  const suffix = `.litecode/${file}`;
  return n === suffix || n.endsWith(`/${suffix}`);
}

function splitLayer(items: McpServerItem[]): {
  defs: McpDefItem[];
  runtime: Record<string, McpRuntimeItem>;
} {
  const defs: McpDefItem[] = [];
  const runtime: Record<string, McpRuntimeItem> = {};
  for (const item of items) {
    const { status, tools, error, ...def } = item;
    defs.push(def);
    runtime[item.id] = { status, tools, error };
  }
  return { defs, runtime };
}

export function splitMcpListing(list: LayeredList<McpServerItem>): {
  mcpDefs: LayeredList<McpDefItem>;
  mcpRuntime: LayeredMcpRuntime;
} {
  const global = splitLayer(list.global ?? []);
  const workspace = splitLayer(list.workspace ?? []);
  return {
    mcpDefs: { global: global.defs, workspace: workspace.defs },
    mcpRuntime: { global: global.runtime, workspace: workspace.runtime },
  };
}

export function mergeLayeredMcp(
  defs: LayeredList<McpDefItem> | null,
  runtime: LayeredMcpRuntime | null,
): LayeredList<McpServerItem> {
  if (!defs) return { global: [], workspace: [] };
  const rt = runtime ?? { global: {}, workspace: {} };
  return {
    global: defs.global.map((d) => ({ ...d, ...(rt.global[d.id] ?? {}) })),
    workspace: defs.workspace.map((d) => ({ ...d, ...(rt.workspace[d.id] ?? {}) })),
  };
}
