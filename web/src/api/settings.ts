import { apiFetch } from "./auth";

export type AgentRole = "primary" | "subagent" | "hidden";
export type ToolPreset = "ALL" | "SAFE";
export type AvailableKind = "core" | "engine" | "custom" | "mcp";
export type ToolOrigin = "builtin" | "global" | "workspace";
export type ToolScope = "global" | "workspace";
export type ThinkingMode = "enabled" | "disabled";
export type ReasoningEffort = "high" | "max";

export type EngineWarmupState =
  | "idle"
  | "warming"
  | "warm"
  | "failed"
  | "stopped";

export interface EngineStatus {
  /** Persisted engine intent; runtime availability is `state === "warm"`. */
  desired?: boolean;
  state?: EngineWarmupState | null;
  error?: string | null;
}

export interface SettingsSummary {
  revision: number;
  ready_provider_count?: number;
  provider_endpoint: string | null;
  model_count: number;
  agent_count: number;
  catalog_count: number;
  log_level: string | null;
  effective_next_turn: boolean;
  restart_required: boolean;
  /** Present when provider → model → required agents are incomplete. */
  setup_guidance?: string | null;
}

export type FieldType =
  | "string"
  | "secret"
  | "number"
  | "boolean"
  | "enum"
  | "string_list";

export interface FieldSchema {
  name: string;
  label: string;
  type: FieldType;
  required: boolean;
  options?: string[] | null;
}

export interface AdapterDescriptor {
  id: string;
  label: string;
  provider_fields: FieldSchema[];
  model_fields: FieldSchema[];
  /** Official host for closed adapters (DeepSeek / MiMo). Open adapters omit this. */
  default_endpoint?: string | null;
  /** When true, Settings can refresh model ids from this provider's `/models`. */
  remote_model_catalog?: boolean;
}

export interface ProviderView {
  id: string;
  adapter_id: string;
  label: string;
  endpoint: string | null;
  api_key: string | null;
  auth: string;
}

export interface ProviderConnectionConfig {
  endpoint: string;
  api_key: string;
  auth: "bearer" | "api_key";
}

export interface ProviderDefinition {
  id: string;
  adapter_id: string;
  label: string;
  config: ProviderConnectionConfig;
}

export interface WebSearchView {
  api_key: string | null;
}

export interface ModelAdapterConfig {
  api_model_id: string;
  context_window: number;
  max_tokens: number;
  thinking_mode?: ThinkingMode | null;
  reasoning_effort?: ReasoningEffort | null;
  json_output?: boolean;
  capabilities: string[];
}

export interface ModelDefinition {
  id: string;
  adapter_id: string;
  provider_ref: string;
  label: string;
  config: ModelAdapterConfig;
}

export interface AgentToolBinding {
  enabled: boolean;
  policy?: ToolPolicy;
  path_mode?: BindingPathMode;
  last_applied_preset?: ToolPreset | null;
  /** Static MCP tool allowlist; null/undefined exposes the server's full catalog. */
  allowed_tools?: string[] | null;
}

export type BindingPathMode = "workspace_only" | "unrestricted";

export type PermissionAction = "allow" | "ask" | "deny";

export interface ToolPolicy {
  default: PermissionAction;
  default_id?: string;
  rules: PolicyRule[];
}

export interface PolicyRule {
  id: string;
  when: ArgMatcher;
  action: PermissionAction;
}

export type ArgMatcher =
  | { kind: "any" }
  | { kind: "arg_equals"; name: string; value: string }
  | { kind: "arg_glob"; name: string; pattern: string }
  | { kind: "path_outside_workspace"; name: string }
  | { kind: "bash_readonly_command" }
  | { kind: "all_of"; matchers: ArgMatcher[] }
  | { kind: "any_of"; matchers: ArgMatcher[] };

export interface AgentProfile {
  role: AgentRole;
  model_ref: string;
  system_prompt: string;
  temperature: number;
  max_steps: number;
  description: string;
  tools: Record<string, AgentToolBinding>;
  allowed_subagents: string[];
}

export interface AvailableTool {
  id: string;
  kind: AvailableKind;
  origin: ToolOrigin;
  overridden?: boolean;
}

export interface ToolSchema {
  type: string;
  properties: Record<string, unknown>;
  required?: string[];
}

export interface CustomToolDefinition {
  name: string;
  description?: string;
  schema: ToolSchema;
  command: string;
  args?: string[];
  timeout?: number;
}

export type McpTransport =
  | { type: "stdio" }
  | { type: "remote"; url: string; headers?: Record<string, string> };

export interface McpServerDefinition {
  command: string;
  args?: string[];
  env?: Record<string, string>;
  transport?: McpTransport;
  timeout?: number;
}

export type McpRunState = "stopped" | "starting" | "running" | "error";

export interface McpToolInfo {
  name: string;
  description: string;
}

export interface McpServerItem extends McpServerDefinition {
  id: string;
  origin?: ToolOrigin;
  status?: McpRunState;
  tools?: McpToolInfo[];
  error?: string | null;
}

export interface McpProbeResult {
  ready: boolean;
  status?: McpRunState;
  tools: McpToolInfo[];
  error?: string | null;
}

export interface LogSettings {
  level: string | null;
}

export interface WorkspaceExcludesLists {
  files_exclude: string[];
  search_exclude: string[];
  watcher_exclude: string[];
  git_ignore: boolean;
  explorer_git_ignore: boolean;
}

export interface WorkspaceExcludes extends WorkspaceExcludesLists {
  defaults: WorkspaceExcludesLists;
}

export interface RevisionResponse {
  revision: number;
  docs: string[];
}

export interface ProviderWriteResponse {
  revision: number;
  docs: string[];
  restart_required: boolean;
}

export interface WorkspaceEnginesDoc {
  version: number;
  lsp: { desired: boolean; servers: string[] };
  retrieval: { desired: boolean };
}

interface ApiOk<T> {
  ok: true;
  data?: T;
}

interface ApiOkFlat {
  ok: true;
  [key: string]: unknown;
}

interface ApiErr {
  ok: false;
  error: string;
}

type ApiResult<T> = ApiOk<T> | ApiOkFlat | ApiErr;

export class SettingsApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message?: string) {
    super(message ?? code);
    this.name = "SettingsApiError";
    this.status = status;
    this.code = code;
  }

  get isTurnBlocked(): boolean {
    return this.status === 409 && this.code === "turn_in_progress";
  }
}

async function parseJson<T>(res: Response): Promise<T> {
  let body: ApiResult<T>;
  try {
    body = (await res.json()) as ApiResult<T>;
  } catch {
    throw new SettingsApiError(res.status, "invalid_response", `HTTP ${res.status}`);
  }

  if (!body.ok) {
    throw new SettingsApiError(res.status, body.error, body.error);
  }

  // RS settings API flattens payload at top level (`#[serde(flatten)]`), not under `data`.
  if ("data" in body && body.data !== undefined) {
    return body.data as unknown as T;
  }
  const { ok: _ok, ...rest } = body as ApiOkFlat;
  return rest as unknown as T;
}

async function requestJson<T>(
  url: string,
  init?: RequestInit,
): Promise<T> {
  const res = await apiFetch(url, init);
  if (!res.ok) {
    try {
      await parseJson<T>(res);
    } catch (err) {
      if (err instanceof SettingsApiError) throw err;
      throw new SettingsApiError(res.status, "request_failed", `HTTP ${res.status}`);
    }
  }
  return parseJson<T>(res);
}

export async function getSettingsSummary(): Promise<SettingsSummary> {
  return requestJson<SettingsSummary>("/api/settings");
}

export async function getAdapters(): Promise<AdapterDescriptor[]> {
  const data = await requestJson<{ adapters: AdapterDescriptor[] }>(
    "/api/settings/adapters",
  );
  return data.adapters;
}

export async function getProviders(): Promise<Record<string, ProviderView>> {
  const data = await requestJson<{ providers: Record<string, ProviderView> }>(
    "/api/settings/providers",
  );
  return data.providers;
}

export async function getProviderModels(providerId: string): Promise<string[]> {
  const data = await requestJson<{ ids: string[] }>(
    `/api/settings/providers/${encodeURIComponent(providerId)}/models`,
  );
  return data.ids ?? [];
}

export async function putProviders(
  providers: Record<string, ProviderDefinition>,
): Promise<ProviderWriteResponse> {
  return requestJson<ProviderWriteResponse>("/api/settings/providers", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ providers }),
  });
}

export async function getWebSearch(): Promise<WebSearchView> {
  return requestJson<WebSearchView>("/api/settings/websearch");
}

export async function putWebSearch(body: {
  api_key?: string;
}): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>("/api/settings/websearch", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export async function getModels(): Promise<Record<string, ModelDefinition>> {
  const data = await requestJson<{ models: Record<string, ModelDefinition> }>(
    "/api/settings/models",
  );
  return data.models;
}

export async function putModels(
  models: Record<string, ModelDefinition>,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>("/api/settings/models", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ models }),
  });
}

export async function getAgent(id: string): Promise<AgentProfile> {
  return requestJson<AgentProfile>(`/api/settings/agents/${encodeURIComponent(id)}`);
}

export async function putAgent(
  id: string,
  profile: AgentProfile,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/agents/${encodeURIComponent(id)}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(withSyncedToolSeries(profile)),
    },
  );
}

export async function applyAgentToolPreset(
  agentId: string,
  preset: ToolPreset,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/agents/${encodeURIComponent(agentId)}/tools/apply-preset`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ preset }),
    },
  );
}

export async function getAvailableTools(): Promise<AvailableTool[]> {
  const data = await requestJson<{ tools: AvailableTool[] }>(
    "/api/settings/available-tools",
  );
  return data.tools ?? [];
}

function scopeQuery(scope: ToolScope = "global"): string {
  return scope === "workspace" ? "?scope=workspace" : "?scope=global";
}

export interface LayeredList<T> {
  global: T[];
  workspace: T[];
}

export async function getCustomTools(): Promise<LayeredList<CustomToolDefinition>> {
  const data = await requestJson<{
    global?: CustomToolDefinition[];
    workspace?: CustomToolDefinition[];
    custom_tools?: CustomToolDefinition[];
  }>("/api/settings/custom-tools");
  if (data.global || data.workspace) {
    return { global: data.global ?? [], workspace: data.workspace ?? [] };
  }
  return { global: data.custom_tools ?? [], workspace: [] };
}

export async function putCustomTool(
  id: string,
  def: CustomToolDefinition,
  scope: ToolScope = "global",
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/custom-tools/${encodeURIComponent(id)}${scopeQuery(scope)}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(def),
    },
  );
}

export async function deleteCustomTool(
  id: string,
  scope: ToolScope = "global",
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/custom-tools/${encodeURIComponent(id)}${scopeQuery(scope)}`,
    { method: "DELETE" },
  );
}

export interface McpRuntimeSnapshot {
  status?: McpRunState;
  tools?: McpToolInfo[];
  error?: string | null;
}

export async function getMcpServers(): Promise<LayeredList<McpServerItem>> {
  const data = await requestJson<{
    global?: Array<McpServerDefinition & { id: string; origin?: ToolOrigin }>;
    workspace?: Array<McpServerDefinition & { id: string; origin?: ToolOrigin }>;
    runtime?: {
      global?: Record<string, McpRuntimeSnapshot>;
      workspace?: Record<string, McpRuntimeSnapshot>;
    };
  }>("/api/settings/mcp-servers");
  const runtime = data.runtime ?? { global: {}, workspace: {} };
  return {
    global: attachMcpRuntime(data.global ?? [], runtime.global ?? {}),
    workspace: attachMcpRuntime(data.workspace ?? [], runtime.workspace ?? {}),
  };
}

function attachMcpRuntime(
  defs: Array<McpServerDefinition & { id: string; origin?: ToolOrigin }>,
  runtime: Record<string, McpRuntimeSnapshot>,
): McpServerItem[] {
  return defs.map((def) => ({ ...def, ...(runtime[def.id] ?? {}) }));
}

export async function putMcpServer(
  id: string,
  def: McpServerDefinition,
  scope: ToolScope = "global",
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/mcp-servers/${encodeURIComponent(id)}${scopeQuery(scope)}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(def),
    },
  );
}

export async function deleteMcpServer(
  id: string,
  scope: ToolScope = "global",
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/mcp-servers/${encodeURIComponent(id)}${scopeQuery(scope)}`,
    { method: "DELETE" },
  );
}

export async function startMcpServer(
  id: string,
  scope: ToolScope = "global",
): Promise<McpProbeResult> {
  return requestJson<McpProbeResult>(
    `/api/settings/mcp-servers/${encodeURIComponent(id)}/start${scopeQuery(scope)}`,
    { method: "POST" },
  );
}

export async function restartMcpServer(
  id: string,
  scope: ToolScope = "global",
): Promise<McpProbeResult> {
  return requestJson<McpProbeResult>(
    `/api/settings/mcp-servers/${encodeURIComponent(id)}/restart${scopeQuery(scope)}`,
    { method: "POST" },
  );
}

export async function stopMcpServer(
  id: string,
  scope: ToolScope = "global",
): Promise<McpProbeResult> {
  return requestJson<McpProbeResult>(
    `/api/settings/mcp-servers/${encodeURIComponent(id)}/stop${scopeQuery(scope)}`,
    { method: "POST" },
  );
}

export async function getLog(): Promise<LogSettings> {
  return requestJson<LogSettings>("/api/settings/log");
}

export async function putLog(level: string | null): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>("/api/settings/log", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ level }),
  });
}

export async function getExcludes(): Promise<WorkspaceExcludes> {
  return requestJson<WorkspaceExcludes>("/api/settings/excludes");
}

export async function putExcludes(
  body: WorkspaceExcludesLists,
): Promise<WorkspaceExcludes> {
  return requestJson<WorkspaceExcludes>("/api/settings/excludes", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export async function getEnginesDoc(): Promise<WorkspaceEnginesDoc> {
  return requestJson<WorkspaceEnginesDoc>("/api/settings/engines");
}

export async function putEnginesDoc(
  file: WorkspaceEnginesDoc,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>("/api/settings/engines", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(file),
  });
}

/** Tools with fixed behavior — only `enabled`, no preset (CONFIG §2.5). */
export const NONE_TOOL_IDS = new Set([
  "plan",
  "todo",
  "subagent_launch",
]);

/** MCP catalog ids (`mcp_*`) have no ALL/SAFE — bind is on/off only. */
export function isMcpCatalogTool(toolId: string): boolean {
  return toolId.startsWith("mcp_");
}

export function isConfigurableTool(toolId: string): boolean {
  return !NONE_TOOL_IDS.has(toolId) && !isMcpCatalogTool(toolId);
}

export interface AgentListItem {
  id: string;
  role: AgentRole;
  description: string;
  allowed_subagents: string[];
}

export const PROTECTED_AGENT_IDS = new Set(["default", "compaction"]);

export const SUBAGENT_SERIES_TOOL_IDS = new Set([
  "subagent_launch",
]);

export function isSubagentBindableTool(entry: AvailableTool): boolean {
  return !SUBAGENT_SERIES_TOOL_IDS.has(entry.id);
}

/** Tools that form one closed loop: enable/disable together. */
export const BASH_SERIES_TOOL_IDS = ["bash", "wait_shell", "kill_shell"] as const;

const TOOL_ENABLE_SERIES: readonly (readonly string[])[] = [BASH_SERIES_TOOL_IDS];

export function toolEnableSeries(toolId: string): readonly string[] | null {
  for (const series of TOOL_ENABLE_SERIES) {
    if (series.includes(toolId)) return series;
  }
  return null;
}

function defaultSeriesBinding(toolId: string): AgentToolBinding {
  return {
    enabled: false,
    last_applied_preset:
      NONE_TOOL_IDS.has(toolId) || isMcpCatalogTool(toolId) ? null : "ALL",
  };
}

/** Toggle `toolId`; if it belongs to a series, apply `enabled` to every member. */
export function applyToolEnabled(
  tools: Record<string, AgentToolBinding>,
  toolId: string,
  enabled: boolean,
): Record<string, AgentToolBinding> {
  const ids = toolEnableSeries(toolId) ?? [toolId];
  const next = { ...tools };
  for (const id of ids) {
    const current = next[id] ?? defaultSeriesBinding(id);
    next[id] = { ...current, enabled };
  }
  return next;
}

/** Persist-time sync: if any series member is on, all members are on (and present). */
export function syncToolEnableSeries(
  tools: Record<string, AgentToolBinding>,
): Record<string, AgentToolBinding> {
  const next = { ...tools };
  for (const series of TOOL_ENABLE_SERIES) {
    const anyPresent = series.some((id) => id in next);
    if (!anyPresent) continue;
    const anyEnabled = series.some((id) => next[id]?.enabled === true);
    for (const id of series) {
      const current = next[id] ?? defaultSeriesBinding(id);
      next[id] = { ...current, enabled: anyEnabled };
    }
  }
  return next;
}

export function withSyncedToolSeries(profile: AgentProfile): AgentProfile {
  return { ...profile, tools: syncToolEnableSeries(profile.tools) };
}

export function isProtectedAgent(id: string): boolean {
  return PROTECTED_AGENT_IDS.has(id);
}

export async function deleteAgent(id: string): Promise<RevisionResponse> {
  const res = await apiFetch(`/api/settings/agents/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    try {
      await parseJson<RevisionResponse>(res);
    } catch (err) {
      if (err instanceof SettingsApiError) throw err;
      throw new SettingsApiError(res.status, "request_failed", `HTTP ${res.status}`);
    }
  }
  return parseJson<RevisionResponse>(res);
}

export function modelOptionLabel(model: ModelDefinition): string {
  return model.label.trim() || model.id;
}

export async function listAgents(): Promise<AgentListItem[]> {
  const data = await requestJson<{ agents: AgentListItem[] }>("/api/settings/agents");
  return data.agents;
}

/** All agents from DB (includes hidden profiles like compaction). */
export function isAgentVisible(role: AgentRole): boolean {
  return role === "primary" || role === "subagent" || role === "hidden";
}

/** Hidden agents configurable in settings (model only). */
export function isHiddenSettingsAgent(id: string, role: AgentRole): boolean {
  return role === "hidden" && id === "compaction";
}

const AGENT_IDS_STORAGE_KEY = "litecode:settingsAgentIds";

export function storeAgentIds(ids: string[]): void {
  sessionStorage.setItem(AGENT_IDS_STORAGE_KEY, JSON.stringify([...new Set(ids)]));
}

/** Load agent ids from settings API (primary, subagent, and hidden compaction). */
export async function loadSettingsAgentIds(): Promise<string[]> {
  const agents = await listAgents();
  const ids = agents
    .filter(
      (a) =>
        a.role === "primary" ||
        a.role === "subagent" ||
        isHiddenSettingsAgent(a.id, a.role),
    )
    .map((a) => a.id);
  const unique = [...new Set(ids)].sort((a, b) => {
    if (a === "default") return -1;
    if (b === "default") return 1;
    if (a === "compaction") return -1;
    if (b === "compaction") return 1;
    return a.localeCompare(b);
  });
  storeAgentIds(unique.length > 0 ? unique : ["default"]);
  return unique.length > 0 ? unique : ["default"];
}
