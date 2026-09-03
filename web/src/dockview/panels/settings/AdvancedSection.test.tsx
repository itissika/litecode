import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "../../../stores/settingsStore";
import { AdvancedSection } from "./AdvancedSection";

describe("AdvancedSection persist UX", () => {
  const saveLog = vi.fn(async () => undefined);
  const saveWebSearch = vi.fn(async () => undefined);

  beforeEach(() => {
    saveLog.mockClear();
    saveWebSearch.mockClear();
    useSettingsStore.setState({
      log: { level: "info" },
      websearch: { api_key: "" },
      persistByDoc: {},
      saveLog,
      saveWebSearch,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not PUT on mount", async () => {
    vi.useFakeTimers();
    render(<AdvancedSection />);
    await vi.advanceTimersByTimeAsync(400);
    expect(saveLog).not.toHaveBeenCalled();
    expect(saveWebSearch).not.toHaveBeenCalled();
  });

  it("saves websearch independently of log", async () => {
    vi.useFakeTimers();
    render(<AdvancedSection />);
    fireEvent.change(screen.getByPlaceholderText("optional"), {
      target: { value: "exa-test-key" },
    });
    await vi.advanceTimersByTimeAsync(400);
    expect(saveWebSearch).toHaveBeenCalledWith({
      api_key: "exa-test-key",
    });
    expect(saveLog).not.toHaveBeenCalled();
  });
});
