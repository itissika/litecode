import { apiFetch } from "./auth";

export type AgentRole = "primary" | "subagent" | "hidden";
export type ToolPreset = "ALL" | "SAFE";
export type AvailableKind = "core" | "engine" | "custom" | "mcp";
export type ToolOrigin = "builtin" | "global" | "workspace";
export type ToolScope = "global" | "workspace";

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
  /** Catalog providers that hold a credential. */
  configured_provider_count: number;
  /** Models selectable right now (their provider has a credential). */
  active_model_count: number;
  agent_count: number;
  log_level: string | null;
  effective_next_turn: boolean;
  restart_required: boolean;
  /** Present when provider → model → required agents are incomplete. */
  setup_guidance?: string | null;
}

/** Wire shape of one entry from `provider-catalog.toml` (backend-owned). */
export interface CatalogModelDto {
  /** Stable composite reference `{provider_id}/{model_id}`; sessions store this. */
  ref: string;
  /** Wire model id — may itself contain `/` (e.g. `deepseek/deepseek-v4-flash`). */
  id: string;
  /** Display label; falls back to `id` in the catalog. */
  label: string;
  provider_id: string;
  provider_name: string;
  context_window: number;
  context_window_max: number;
  modalities: string[];
  tool_call: boolean;
  json_output: boolean;
  /** The Models page switched it on. Off models stay out of every picker. */
  enabled: boolean;
}

export type EndpointType = "responses" | "chat_completions" | "messages";

export interface CatalogProviderDto {
  id: string;
  name: string;
  /** Catalog providers the user may add a credential for. */
  visible: boolean;
  /** A credential exists. */
  configured: boolean;
  masked_api_key: string | null;
  endpoint: string | null;
  endpoint_type: EndpointType;
  models: CatalogModelDto[];
}

/** `GET /api/settings/llm` — the single read model for every LLM surface. */
export interface LlmSettings {
  /** Absolute path of the user-editable catalog, for diagnostics. */
  catalog_path: string;
  revision: number;
  providers: CatalogProviderDto[];
  /** Only models whose provider holds a credential. */
  active_models: CatalogModelDto[];
}

/**
 * Split a composite `{provider_id}/{model_id}` ref on the **first** `/` only:
 * model ids may legitimately contain `/`, so a naive `split("/")` would
 * mis-attribute `commandcode/deepseek/deepseek-v4-flash`.
 */
export function splitModelRef(ref: string): {
  providerId: string;
  modelId: string;
} {
  const at = ref.indexOf("/");
  if (at <= 0) return { providerId: "", modelId: ref };
  return { providerId: ref.slice(0, at), modelId: ref.slice(at + 1) };
}

export function modelRefLabel(model: CatalogModelDto): string {
  return model.label.trim() || model.id;
}

export interface WebSearchView {
  api_key: string | null;
}

export interface AgentToolBinding {
  enabled: boolean;
  /** Permission intent; policy/path are derived by the backend at call time. */
  preset?: ToolPreset | null;
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

export type PermissionSurface = "preset" | "fixed";

/** Role/depth scope supplied by the backend. */
export type BindingScope = "all" | "primary_root_only";

/** Declared call surface. `code` is reached through `run_code`, not a user mode. */
export type CallSurface = "direct" | "code";

/** Code transport. It is not a settings tool switch. */
export const CODE_TRANSPORT_TOOL_ID = "run_code";

export interface AvailableTool {
  id: string;
  kind: AvailableKind;
  origin: ToolOrigin;
  overridden?: boolean;
  /** Backend declaration: `preset` shows ALL/SAFE; `fixed` is bind only. */
  permission_surface?: PermissionSurface;
  /** Declared call surface from the available-tools payload. */
  surface?: CallSurface;
  /** Concrete model functions behind this card. MCP members load after handshake. */
  members?: string[];
  /** Runtime projection; definitions without an engine omit it. */
  status?: AvailableStatus;
  /** Human label for a grouped custom suite. Other cards use their id. */
  label?: string;
  /** Role/depth scope. `primary_root_only` is hidden from subagent cards. */
  scope?: BindingScope;
}

export type AvailableStatus = "ready" | "warming" | "disabled" | "failed" | "down";

export function isCodeSurface(tool: { surface?: CallSurface | null }): boolean {
  return tool.surface === "code";
}

/** Bind cards. The code transport is omitted even if a payload still lists it. */
export function settingsToolSwitches(
  tools: readonly AvailableTool[],
): AvailableTool[] {
  return tools.filter((tool) => tool.id !== CODE_TRANSPORT_TOOL_ID);
}

export interface ToolSchema {
  type: string;
  properties: Record<string, unknown>;
  required?: string[];
}

export type PermissionActionName = "allow" | "ask" | "deny";

export interface ToolPermissionRule {
  id: string;
  when: { kind: string } & Record<string, unknown>;
  action: PermissionActionName;
}

export interface CustomToolDefinition {
  name: string;
  description?: string;
  /** Standalone tools only; suite members omit it and inherit from the suite. */
  surface?: CallSurface;
  schema: ToolSchema;
  /** Code-surface result schema; required on code, forbidden on direct. */
  output_schema?: Record<string, unknown>;
  command: string;
  args?: string[];
  timeout?: number;
  /** Standalone only: Agents can switch ALL/SAFE. Suite members leave it empty. */
  rules?: ToolPermissionRule[];
  /** Suite member reference; the suite owns surface and rules. */
  suite?: string | null;
}

export interface CustomSuiteDefinition {
  surface: CallSurface;
  label?: string;
  rules?: ToolPermissionRule[];
}

export interface CustomToolDiagnostic {
  source: string;
  file?: string | null;
  tool?: string | null;
  suite?: string | null;
  field: string;
  actual?: string | null;
  expected: string;
  fix: string;
  message: string;
}

export interface ToolRejection {
  name: string;
  reason: string;
}

export interface CustomToolsView {
  global: CustomToolDefinition[];
  workspace: CustomToolDefinition[];
  suites: {
    global: Record<string, CustomSuiteDefinition>;
    workspace: Record<string, CustomSuiteDefinition>;
  };
  diagnostics: CustomToolDiagnostic[];
  code_rejections: ToolRejection[];
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
  code_index_extensions: string[];
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
    throw new SettingsApiError(
      res.status,
      "invalid_response",
      `HTTP ${res.status}`,
    );
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

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(url, init);
  if (!res.ok) {
    try {
      await parseJson<T>(res);
    } catch (err) {
      if (err instanceof SettingsApiError) throw err;
      throw new SettingsApiError(
        res.status,
        "request_failed",
        `HTTP ${res.status}`,
      );
    }
  }
  return parseJson<T>(res);
}

export async function getSettingsSummary(): Promise<SettingsSummary> {
  return requestJson<SettingsSummary>("/api/settings");
}

/** Catalog + credential state; the only LLM read model in the UI. */
export async function getLlmSettings(): Promise<LlmSettings> {
  return requestJson<LlmSettings>("/api/settings/llm");
}

/** Store (or replace) one provider credential. Never writes the catalog file. */
export async function putProviderKey(
  providerId: string,
  apiKey: string,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/providers/${encodeURIComponent(providerId)}/key`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ api_key: apiKey }),
    },
  );
}

/** Drop one provider credential; the catalog provider is never deleted. */
export async function deleteProviderKey(
  providerId: string,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/providers/${encodeURIComponent(providerId)}/key`,
    { method: "DELETE" },
  );
}

/**
 * Switch one catalog model on or off for every picker.
 *
 * The catalog file is untouched: enablement is user state, and the model ref
 * travels in the body because a ref contains `/` twice over
 * (`commandcode/deepseek/deepseek-v4-flash`).
 */
export async function putModelEnabled(
  modelRef: string,
  enabled: boolean,
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>("/api/settings/models/enabled", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ref: modelRef, enabled }),
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

export async function getAgent(id: string): Promise<AgentProfile> {
  return requestJson<AgentProfile>(
    `/api/settings/agents/${encodeURIComponent(id)}`,
  );
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
      body: JSON.stringify(profile),
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

export async function getCustomTools(): Promise<CustomToolsView> {
  const data = await requestJson<{
    global?: CustomToolDefinition[];
    workspace?: CustomToolDefinition[];
    custom_tools?: CustomToolDefinition[];
    suites?: {
      global?: Record<string, CustomSuiteDefinition>;
      workspace?: Record<string, CustomSuiteDefinition>;
    };
    diagnostics?: CustomToolDiagnostic[];
    code_rejections?: ToolRejection[];
  }>("/api/settings/custom-tools");
  return {
    global: data.global ?? data.custom_tools ?? [],
    workspace: data.workspace ?? [],
    suites: {
      global: data.suites?.global ?? {},
      workspace: data.suites?.workspace ?? {},
    },
    diagnostics: data.diagnostics ?? [],
    code_rejections: data.code_rejections ?? [],
  };
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

export async function putCustomSuite(
  id: string,
  def: CustomSuiteDefinition,
  scope: ToolScope = "global",
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/custom-suites/${encodeURIComponent(id)}${scopeQuery(scope)}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(def),
    },
  );
}

export async function deleteCustomSuite(
  id: string,
  scope: ToolScope = "global",
): Promise<RevisionResponse> {
  return requestJson<RevisionResponse>(
    `/api/settings/custom-suites/${encodeURIComponent(id)}${scopeQuery(scope)}`,
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
    workspace?: Array<
      McpServerDefinition & { id: string; origin?: ToolOrigin }
    >;
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

/** True when the backend card declares an ALL/SAFE dial. */
export function isConfigurableTool(tool: {
  permission_surface?: PermissionSurface;
}): boolean {
  return tool.permission_surface === "preset";
}

export function presetToolIds(
  tools: readonly { id: string; permission_surface?: PermissionSurface }[],
): Set<string> {
  return new Set(
    tools.filter((tool) => tool.permission_surface === "preset").map((tool) => tool.id),
  );
}

export interface AgentListItem {
  id: string;
  role: AgentRole;
  description: string;
  allowed_subagents: string[];
}

export const PROTECTED_AGENT_IDS = new Set(["default"]);

/** One card writes one binding row. */
export function applyToolEnabled(
  tools: Record<string, AgentToolBinding>,
  toolId: string,
  enabled: boolean,
  presetIds: ReadonlySet<string> = new Set(),
): Record<string, AgentToolBinding> {
  const current = tools[toolId] ?? {
    enabled: false,
    preset: presetIds.has(toolId) ? "ALL" : null,
  };
  return { ...tools, [toolId]: { ...current, enabled } };
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
      throw new SettingsApiError(
        res.status,
        "request_failed",
        `HTTP ${res.status}`,
      );
    }
  }
  return parseJson<RevisionResponse>(res);
}

export async function listAgents(): Promise<AgentListItem[]> {
  const data = await requestJson<{ agents: AgentListItem[] }>(
    "/api/settings/agents",
  );
  return data.agents;
}

const AGENT_IDS_STORAGE_KEY = "litecode:settingsAgentIds";

export function storeAgentIds(ids: string[]): void {
  sessionStorage.setItem(
    AGENT_IDS_STORAGE_KEY,
    JSON.stringify([...new Set(ids)]),
  );
}

/** Load configurable agent ids from the settings API. */
export async function loadSettingsAgentIds(): Promise<string[]> {
  const agents = await listAgents();
  const ids = agents
    .filter((a) => a.role === "primary" || a.role === "subagent")
    .map((a) => a.id);
  const unique = [...new Set(ids)].sort((a, b) => {
    if (a === "default") return -1;
    if (b === "default") return 1;
    return a.localeCompare(b);
  });
  storeAgentIds(unique.length > 0 ? unique : ["default"]);
  return unique.length > 0 ? unique : ["default"];
}
