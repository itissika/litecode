import { apiFetch } from "./auth";

export type AppStatus =
  | "stopped"
  | "not_installed"
  | "installing"
  | "installed"
  | "starting"
  | "running"
  | "stopping"
  | "crashed"
  | "error"
  | "uninstalling";

export type AppCategory =
  | "data"
  | "analysis"
  | "document"
  | "design"
  | "image"
  | "video"
  | "audio"
  | "task"
  | "general";

export type AppSource = "builtin" | "global" | "workspace";

export interface AppInstallProgress {
  phase: string;
  percent: number | null;
  message: string | null;
}

export interface AppInstallError {
  message: string;
  log_path: string | null;
  tail: string | null;
}

/** Managed install state. Only bundled apps that ship one have this at all. */
export interface AppInstallState {
  installed: boolean;
  can_install: boolean;
  can_uninstall: boolean;
  progress: AppInstallProgress | null;
  error: AppInstallError | null;
}

/** Offline documentation for a bundled app, in both UI languages. */
export interface AppDocs {
  en: string;
  zh: string;
}

export interface AppView {
  id: string;
  name: string;
  description: string;
  /** One-line description by locale; `description` is the fallback. */
  description_i18n: Record<string, string>;
  category: AppCategory;
  icon: string;
  /** Official repository and optional docs link; both may be absent. */
  repo: string | null;
  docs_url: string | null;
  docs: AppDocs | null;
  source: AppSource;
  view_key: string;
  status: AppStatus;
  running: boolean;
  url: string | null;
  /** Set when the app is a panel LiteCode paints itself ("gallery"). */
  builtin_view: string | null;
  port: number | null;
  can_start: boolean;
  can_open: boolean;
  trusted: boolean;
  definition_hash: string | null;
  trust_preview: string | null;
  permissions: string[];
  single_instance: boolean;
  /** null for registered apps: they bring their own dependencies. */
  install: AppInstallState | null;
  runtime_error: string | null;
}

export interface AppsRegistryError {
  /** "error" keeps the entry out; "warning" only needs to be shown. */
  severity: "error" | "warning";
  source: string;
  file: string;
  app_id: string | null;
  message: string;
}

/** External recommendation metadata. It has no launch and cannot be started. */
export interface RecommendationView {
  id: string;
  name: string;
  description: string;
  description_i18n: Record<string, string>;
  category: AppCategory;
  icon: string;
  repo: string;
}

export interface AppsSnapshot {
  apps: AppView[];
  recommendations: RecommendationView[];
  registry_paths: { global: string; workspace: string };
  registry_errors: AppsRegistryError[];
}

interface ApiOk<T> {
  ok: true;
  data: T;
}

interface ApiErr {
  ok: false;
  error: string;
}

async function parse<T>(res: Response): Promise<T> {
  const body = (await res.json()) as ApiOk<T> | ApiErr;
  if (!body.ok) throw new Error(body.error || `HTTP ${res.status}`);
  return body.data;
}

export async function fetchApps(): Promise<AppsSnapshot> {
  // The panel polls this endpoint; never let a browser cache serve a stale
  // install state after the server has moved on.
  return parse<AppsSnapshot>(
    await apiFetch("/api/workspace/apps", { cache: "no-store" }),
  );
}

async function postApp(id: string, action: string, init?: RequestInit): Promise<void> {
  const res = await apiFetch(
    `/api/workspace/apps/${encodeURIComponent(id)}/${action}`,
    { method: "POST", ...init },
  );
  await parse<unknown>(res);
}

export async function installApp(id: string): Promise<void> {
  await postApp(id, "install");
}

export async function startApp(id: string): Promise<void> {
  await postApp(id, "start");
}

export async function stopApp(id: string): Promise<void> {
  await postApp(id, "stop");
}

export async function uninstallApp(id: string): Promise<void> {
  await postApp(id, "uninstall");
}

export async function trustApp(id: string, definitionHash: string): Promise<void> {
  await postApp(id, "trust", {
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ definition_hash: definitionHash }),
  });
}
