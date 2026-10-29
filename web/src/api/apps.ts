import { apiFetch } from "./auth";

export type AppStatus =
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
export type InstallScope = "global" | "workspace";

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

export interface AppView {
  id: string;
  name: string;
  description: string;
  /** One-line description by locale; `description` is the fallback. */
  description_i18n: Record<string, string>;
  summary: string;
  category: AppCategory;
  icon: string;
  homepage: string | null;
  source: AppSource;
  install_scope: InstallScope;
  install_dir: string;
  data_dir: string;
  view_key: string;
  status: AppStatus;
  installed: boolean;
  running: boolean;
  url: string | null;
  /** Set when the app is a panel LiteCode paints itself ("gallery"). */
  builtin_view: string | null;
  port: number | null;
  required_bytes: number | null;
  size_bytes: number | null;
  can_install: boolean;
  can_uninstall: boolean;
  can_start: boolean;
  can_open: boolean;
  trusted: boolean;
  definition_hash: string | null;
  trust_preview: string | null;
  permissions: string[];
  data_scope: "workspace" | "app";
  single_instance: boolean;
  install_progress: AppInstallProgress | null;
  install_error: AppInstallError | null;
  runtime_error: string | null;
}

export interface AppsRegistryError {
  source: string;
  file: string;
  app_id: string | null;
  message: string;
}

export interface AppsSnapshot {
  apps: AppView[];
  available_bytes: number | null;
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
  return parse<AppsSnapshot>(await apiFetch("/api/workspace/apps"));
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

export async function fetchAppReadme(id: string): Promise<string> {
  const res = await apiFetch(
    `/api/workspace/apps/${encodeURIComponent(id)}/readme`,
  );
  const data = await parse<{ id: string; text: string }>(res);
  return data.text;
}
