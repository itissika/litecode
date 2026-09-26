import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SettingsSummary } from "../api/settings";
import { useSettingsStore } from "../stores/settingsStore";
import { useToastStore } from "../stores/toastStore";
import {
  KEY_STEER_DELAY_MS,
  NO_KEY_TOAST_CHANNEL,
  READY_TOAST_CHANNEL,
  areClicksLocked,
  noteHello,
  noteSummary,
  resetFirstRunForTests,
} from "./firstRun";

function summary(over: Partial<SettingsSummary> = {}): SettingsSummary {
  return {
    revision: 1,
    configured_provider_count: 0,
    active_model_count: 0,
    agent_count: 1,
    log_level: null,
    effective_next_turn: true,
    restart_required: false,
    ...over,
  };
}

const originalOpenSettings = useSettingsStore.getState().openSettings;
const originalSetSection = useSettingsStore.getState().setSection;
const originalSetSelectedAgentId = useSettingsStore.getState().setSelectedAgentId;

let openSettings: ReturnType<typeof vi.fn>;
let setSection: ReturnType<typeof vi.fn>;
let setSelectedAgentId: ReturnType<typeof vi.fn>;

function toastIds(): string[] {
  return useToastStore.getState().toasts.map((t) => t.id);
}

beforeEach(() => {
  resetFirstRunForTests();
  openSettings = vi.fn();
  setSection = vi.fn().mockResolvedValue(undefined);
  setSelectedAgentId = vi.fn();
  useSettingsStore.setState({
    open: false,
    section: "connection",
    focusAnchor: null,
    summary: null,
    openSettings: openSettings as never,
    setSection: setSection as never,
    setSelectedAgentId: setSelectedAgentId as never,
  });
  useToastStore.setState({ toasts: [] });
});

afterEach(() => {
  resetFirstRunForTests();
  useSettingsStore.setState({
    open: false,
    section: "connection",
    focusAnchor: null,
    summary: null,
    openSettings: originalOpenSettings,
    setSection: originalSetSection,
    setSelectedAgentId: originalSetSelectedAgentId,
  });
  useToastStore.setState({ toasts: [] });
});

describe("no key", () => {
  it("opens the Provider page once and leaves a single emoji note", () => {
    noteHello(summary());
    expect(openSettings).toHaveBeenCalledWith("connection");
    expect(toastIds()).toEqual([NO_KEY_TOAST_CHANNEL]);
    expect(useToastStore.getState().toasts[0]?.icon).toBe("🫠");
  });

  it("stays quiet on later hellos in the same run", () => {
    noteHello(summary());
    noteHello(summary({ revision: 2 }));
    expect(openSettings).toHaveBeenCalledTimes(1);
    expect(toastIds()).toEqual([NO_KEY_TOAST_CHANNEL]);
  });
});

describe("configured", () => {
  it("does nothing for a workspace that is already ready", () => {
    noteHello(summary({ configured_provider_count: 1, active_model_count: 2 }));
    expect(openSettings).not.toHaveBeenCalled();
    expect(toastIds()).toEqual([]);
    expect(areClicksLocked()).toBe(false);
  });

  it("stays quiet when a key exists but every model is switched off", () => {
    noteHello(summary({ configured_provider_count: 1, active_model_count: 0 }));
    expect(openSettings).not.toHaveBeenCalled();
    expect(toastIds()).toEqual([]);
  });
});

describe("key edge", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    resetFirstRunForTests();
    vi.useRealTimers();
  });

  it("celebrates immediately, ignores clicks, then opens Max steps", () => {
    noteHello(summary());
    useSettingsStore.setState({ open: true });

    noteSummary(summary({ configured_provider_count: 1, active_model_count: 2 }));

    expect(toastIds()).toEqual([READY_TOAST_CHANNEL]);
    expect(useToastStore.getState().toasts[0]?.icon).toBe("🎉");
    expect(areClicksLocked()).toBe(true);
    expect(setSection).not.toHaveBeenCalled();

    vi.advanceTimersByTime(KEY_STEER_DELAY_MS - 1);
    expect(setSection).not.toHaveBeenCalled();
    expect(areClicksLocked()).toBe(true);

    vi.advanceTimersByTime(1);
    expect(areClicksLocked()).toBe(false);
    expect(setSelectedAgentId).toHaveBeenCalledWith("default");
    expect(setSection).toHaveBeenCalledWith("agents", { anchor: "max-steps" });

    noteSummary(summary({ configured_provider_count: 2, active_model_count: 3 }));
    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);
    expect(setSection).toHaveBeenCalledTimes(1);
    expect(
      toastIds().filter((id) => id === READY_TOAST_CHANNEL),
    ).toHaveLength(1);
  });

  it("celebrates the same way when the new key has no model switched on", () => {
    noteHello(summary());
    useSettingsStore.setState({ open: true });

    noteSummary(summary({ configured_provider_count: 1, active_model_count: 0 }));

    expect(toastIds()).toEqual([READY_TOAST_CHANNEL]);
    expect(areClicksLocked()).toBe(true);
    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);
    expect(setSection).toHaveBeenCalledWith("agents", { anchor: "max-steps" });
  });

  it("never steers when the dialog is closed by then", () => {
    noteHello(summary());
    useSettingsStore.setState({ open: false });

    noteSummary(summary({ configured_provider_count: 1, active_model_count: 2 }));
    expect(toastIds()).toEqual([READY_TOAST_CHANNEL]);
    expect(areClicksLocked()).toBe(false);
    expect(setSection).not.toHaveBeenCalled();

    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);
    expect(setSection).not.toHaveBeenCalled();

    // Re-opening Settings later must not replay the detour.
    useSettingsStore.setState({ open: true });
    noteSummary(summary({ configured_provider_count: 3, active_model_count: 4 }));
    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);
    expect(setSection).not.toHaveBeenCalled();
  });

  it("drops the detour when Settings closes during the pause", () => {
    noteHello(summary());
    useSettingsStore.setState({ open: true });
    noteSummary(summary({ configured_provider_count: 1, active_model_count: 2 }));

    useSettingsStore.setState({ open: false });
    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);

    expect(setSection).not.toHaveBeenCalled();
    expect(areClicksLocked()).toBe(false);
  });

  it("does not celebrate for a user who already had a key", () => {
    noteHello(summary({ configured_provider_count: 1, active_model_count: 2 }));
    useSettingsStore.setState({ open: true });

    noteSummary(summary({ configured_provider_count: 2, active_model_count: 3 }));
    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);
    expect(toastIds()).toEqual([]);
    expect(setSection).not.toHaveBeenCalled();
    expect(areClicksLocked()).toBe(false);
  });

  it("tracks summary changes pushed by the store", () => {
    noteHello(summary());
    useSettingsStore.setState({ open: true });

    useSettingsStore.setState({
      summary: summary({ configured_provider_count: 1, active_model_count: 2 }),
    });

    expect(toastIds()).toEqual([READY_TOAST_CHANNEL]);
    expect(areClicksLocked()).toBe(true);
    vi.advanceTimersByTime(KEY_STEER_DELAY_MS);
    expect(setSection).toHaveBeenCalledWith("agents", { anchor: "max-steps" });
  });
});
