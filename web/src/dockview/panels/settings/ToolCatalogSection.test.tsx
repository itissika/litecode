import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "../../../stores/settingsStore";
import { ToolCatalogSection } from "./ToolCatalogSection";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("ToolCatalogSection autosave", () => {
  it("persists catalog_enabled when a non-core checkbox is toggled", async () => {
    vi.useFakeTimers();
    const saveToolCatalog = vi.fn(async () => undefined);
    useSettingsStore.setState({
      persistStatus: "idle",
      toolCatalog: {
        grep: {
          id: "grep",
          tier: "optional",
          init_scope: "none",
          readiness: "ready",
          catalog_enabled: false,
        },
      },
      saveToolCatalog,
    });

    render(<ToolCatalogSection />);
    const checkbox = screen.getByRole("checkbox");
    fireEvent.click(checkbox);
    await vi.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    expect(saveToolCatalog).toHaveBeenCalledTimes(1);
    const payload = saveToolCatalog.mock.calls[0][0] as Record<
      string,
      { catalog_enabled: boolean }
    >;
    expect(payload.grep.catalog_enabled).toBe(true);
  });
});
