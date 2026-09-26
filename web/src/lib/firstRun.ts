import type { SettingsSummary } from "../api/settings";
import {
  useSettingsStore,
  type SettingsSection,
} from "../stores/settingsStore";
import { useToastStore } from "../stores/toastStore";

/**
 * First-run combo for an install with no provider key.
 *
 * The golden marker is "is any provider key configured" — the backend already
 * reports it as `SettingsSummary.configured_provider_count`. No key means the
 * agent cannot run, so once per page load LiteCode opens Settings on the
 * Provider page and leaves a single toast. A key that is already there stays
 * silent, including when every model is switched off.
 *
 * When a key lands in this same page (0 → N), celebrate immediately. If
 * Settings is still open, clicks are ignored for one second and then the
 * dialog moves to the default agent's Max steps row. Closing the dialog
 * before that edge, or during the pause, is the user's answer: never reopen
 * it. The "already shown" bits live in module memory. A reload forgets them;
 * the key count is what keeps a configured install quiet.
 *
 * No backend state, no persistence, no protocol field.
 * Opening a chat when the centre grid has no panels is ordinary layout
 * (`lib/centreChat`), not part of this combo.
 */

/** One feature worth a single line of guidance once the workspace is usable. */
export interface ExploreStep {
  id: string;
  emoji: string;
  /** Markdown, one or two short sentences at most. */
  md: string;
  /** Settings page the step lives on. */
  section: SettingsSection;
  /** Row to centre when the step is opened (see SettingsDialog anchors). */
  anchor?: string;
}

/**
 * The explore list. v1 consumes only the max-steps step; the shape is here so
 * later steps are data, not UI.
 */
export const EXPLORE_STEPS: ExploreStep[] = [
  {
    id: "max-steps",
    emoji: "🧗",
    md: "Default **Max steps** is 50 — long tasks stop early. 200 is a saner default.",
    section: "agents",
    anchor: "max-steps",
  },
];

export const NO_KEY_TOAST_CHANNEL = "first-run-no-key";
export const READY_TOAST_CHANNEL = "first-run-ready";

/** Clicks are swallowed for this long between the celebration and the Agents detour. */
export const KEY_STEER_DELAY_MS = 1000;

const NO_KEY_ICON = "🫠";
const NO_KEY_MD =
  "**No provider key yet** — the agent can't run without one.\n\nPaste one under Providers on the left and you're good.";

const READY_ICON = "🎉";
const READY_MD = "**All set — you're ready to chat.**";

type FirstRunPhase = "unknown" | "unconfigured" | "configured";

interface FirstRunState {
  /** A settings page was opened for the user once this run. */
  prompted: boolean;
  /** The post-key detour (Agents → Max steps) already ran or was declined. */
  steered: boolean;
  phase: FirstRunPhase;
}

function initialState(): FirstRunState {
  return {
    prompted: false,
    steered: false,
    phase: "unknown",
  };
}

let state = initialState();
let unsubscribeSummary: (() => void) | null = null;
let steerTimer: ReturnType<typeof setTimeout> | null = null;

let clicksLocked = false;
const clickLockListeners = new Set<() => void>();

/** True while the post-key pause is swallowing pointer input. */
export function areClicksLocked(): boolean {
  return clicksLocked;
}

export function subscribeClickLock(listener: () => void): () => void {
  clickLockListeners.add(listener);
  return () => {
    clickLockListeners.delete(listener);
  };
}

function setClicksLocked(next: boolean): void {
  if (clicksLocked === next) return;
  clicksLocked = next;
  for (const listener of clickLockListeners) listener();
}

function clearSteerTimer(): void {
  if (steerTimer !== null) {
    clearTimeout(steerTimer);
    steerTimer = null;
  }
  setClicksLocked(false);
}

function toast(
  message: string,
  icon: string,
  channel: string,
  durationMs: number,
): void {
  useToastStore.getState().showToast(message, "info", durationMs, channel, {
    icon,
  });
}

function announceConfigured(): void {
  // The "no key" note is stale the moment a key exists. One toast, the celebration.
  useToastStore.getState().dismissToast(NO_KEY_TOAST_CHANNEL);
  toast(READY_MD, READY_ICON, READY_TOAST_CHANNEL, 6000);
  // Steer once. If the dialog is already closed, that is the user's answer —
  // never reopen it. The same applies if they close it during the pause.
  if (state.steered) return;
  state.steered = true;
  if (!useSettingsStore.getState().open) return;
  const step = EXPLORE_STEPS[0];
  if (!step) return;
  setClicksLocked(true);
  steerTimer = setTimeout(() => {
    steerTimer = null;
    setClicksLocked(false);
    const settings = useSettingsStore.getState();
    if (!settings.open) return;
    settings.setSelectedAgentId("default");
    void settings.setSection(step.section, { anchor: step.anchor });
  }, KEY_STEER_DELAY_MS);
}

/**
 * `server/hello` reached us with the freshly fetched summary (see
 * `settingsStore.noteWorkspaceReady`). A null summary means we could not read
 * the key state — do nothing rather than guess.
 */
export function noteHello(summary: SettingsSummary | null): void {
  ensureSummarySubscription();
  if (!summary) return;

  if (summary.configured_provider_count > 0) {
    state.phase = "configured";
    return;
  }

  state.phase = "unconfigured";
  if (!state.prompted) {
    state.prompted = true;
    useSettingsStore.getState().openSettings("connection");
    toast(NO_KEY_MD, NO_KEY_ICON, NO_KEY_TOAST_CHANNEL, 12000);
  }
}

/**
 * Every summary change (`settings/changed`), so a key written from Settings, the
 * CLI or another window all land on the same 0 → N edge.
 */
export function noteSummary(summary: SettingsSummary | null): void {
  if (!summary) return;
  if (summary.configured_provider_count === 0) {
    state.phase = "unconfigured";
    return;
  }
  const wasUnconfigured = state.phase === "unconfigured";
  state.phase = "configured";
  if (wasUnconfigured) announceConfigured();
}

function ensureSummarySubscription(): void {
  if (unsubscribeSummary) return;
  unsubscribeSummary = useSettingsStore.subscribe((next, prev) => {
    if (next.summary !== prev.summary) noteSummary(next.summary);
  });
}

/** Test hook: forget everything this run learned. */
export function resetFirstRunForTests(): void {
  clearSteerTimer();
  state = initialState();
  unsubscribeSummary?.();
  unsubscribeSummary = null;
}
