import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "../../../stores/settingsStore";
import { FilesSection } from "./FilesSection";

vi.mock("../../../stores/treeStore", () => ({
  useTreeStore: {
    getState: () => ({
      refreshAll: vi.fn(async () => undefined),
    }),
  },
}));

const emptyLists = {
  files_exclude: [] as string[],
  search_exclude: [] as string[],
  watcher_exclude: [] as string[],
  git_ignore: true,
  explorer_git_ignore: false,
};

describe("FilesSection", () => {
  const saveExcludes = vi.fn(async () => undefined);

  beforeEach(() => {
    saveExcludes.mockClear();
    useSettingsStore.setState({
      excludes: { ...emptyLists, defaults: emptyLists },
      persistStatus: "idle",
      saveExcludes,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not PUT excludes within 400ms of mount", async () => {
    vi.useFakeTimers();
    render(<FilesSection />);
    await vi.advanceTimersByTimeAsync(400);
    expect(saveExcludes).not.toHaveBeenCalled();
  });
});
