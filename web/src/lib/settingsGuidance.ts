import { useToastStore } from "../stores/toastStore";
import { useSettingsStore } from "../stores/settingsStore";

/**
 * Corner toast for LLM / AI-setup failures (not the bell).
 * Prefers backend `setup_guidance`; otherwise uses the RPC/turn message or a
 * default that names default (primary) and compaction (hidden).
 */
export function toastLlmConfigFailure(fallback?: string): void {
  const guidance = useSettingsStore.getState().summary?.setup_guidance?.trim();
  const message =
    guidance ||
    fallback?.trim() ||
    "AI setup incomplete — assign models to default (primary) and compaction (hidden) in Settings → Agents. Agent runs will fail until this is fixed.";
  useToastStore.getState().showToast(message, "info", 12000);
}
