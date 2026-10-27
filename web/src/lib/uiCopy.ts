import i18n from "../i18n";

/** Shared UI copy used in more than one place (WP-159 §3 / WP-221 i18n). */
export function noModelsHint(): string {
  return i18n.t("settings.models.noModels");
}
export function remoteServeHint(): string {
  return i18n.t("chrome.remoteServeHint");
}

/** @deprecated Prefer noModelsHint() / remoteServeHint(). */
export const NO_MODELS_HINT =
  "No models yet — add a provider key in Settings › Provider.";
export const REMOTE_SERVE_HINT =
  "Connected to a remote litecode serve (no local sidecar)";
