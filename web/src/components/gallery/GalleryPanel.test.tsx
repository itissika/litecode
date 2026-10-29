import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IDockviewPanelProps } from "dockview-react";

import i18n from "../../i18n";
import { fetchMedia, type MediaEntry } from "../../api/workspace";
import { GalleryPanel } from "./GalleryPanel";

vi.mock("../../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/workspace")>();
  return {
    ...actual,
    fetchMedia: vi.fn(),
    fetchMediaThumb: vi.fn(() => new Promise<Blob>(() => {})),
    openWithDefaultApp: vi.fn(),
  };
});

const entries: MediaEntry[] = [
  {
    path: "art/one.png",
    name: "one.png",
    folder: "art",
    kind: "image",
    size: 1200,
    mtime: 1,
  },
  {
    path: "clips/two.mp4",
    name: "two.mp4",
    folder: "clips",
    kind: "video",
    size: 2200,
    mtime: 2,
  },
];

function renderPanel() {
  return render(<GalleryPanel {...({} as IDockviewPanelProps)} />);
}

afterEach(() => {
  cleanup();
});

beforeEach(async () => {
  await i18n.changeLanguage("en");
  vi.mocked(fetchMedia).mockReset();
  vi.mocked(fetchMedia).mockResolvedValue({ entries, truncated: false });
});

describe("GalleryPanel", () => {
  it("lists media and the workspace totals", async () => {
    renderPanel();
    expect(await screen.findByText("one.png")).toBeTruthy();
    expect(screen.getByText("two.mp4")).toBeTruthy();
    expect(screen.getByText("2 items")).toBeTruthy();
    expect(screen.getByText("2 folders")).toBeTruthy();
  });

  it("drills into a folder and filters the grid", async () => {
    renderPanel();
    await screen.findByText("one.png");

    fireEvent.click(screen.getByRole("tab", { name: "Folders" }));
    expect(await screen.findByTitle("art")).toBeTruthy();
    expect(screen.getByTitle("clips")).toBeTruthy();
    // Every entry lives in a folder, so the root grid is empty.
    expect(screen.queryByText("one.png")).toBeNull();

    fireEvent.click(screen.getByTitle("art"));
    expect(screen.getByText("one.png")).toBeTruthy();
    expect(screen.queryByText("two.mp4")).toBeNull();
    expect(screen.getByText("All folders")).toBeTruthy();
  });

  it("groups by media kind", async () => {
    renderPanel();
    await screen.findByText("one.png");

    fireEvent.click(screen.getByRole("tab", { name: "Types" }));
    expect(screen.getByText("Images")).toBeTruthy();
    expect(screen.getByText("Videos")).toBeTruthy();
    expect(screen.queryByText("Audio")).toBeNull();
  });

  it("explains an empty workspace", async () => {
    vi.mocked(fetchMedia).mockResolvedValue({ entries: [], truncated: false });
    renderPanel();
    expect(
      await screen.findByText("No media files in this workspace."),
    ).toBeTruthy();
  });
});
