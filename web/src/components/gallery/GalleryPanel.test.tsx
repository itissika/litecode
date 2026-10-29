import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IDockviewPanelProps } from "dockview-react";

import i18n from "../../i18n";
import {
  fetchMedia,
  openWithDefaultApp,
  readBytes,
  revealInFileManager,
  type MediaEntry,
} from "../../api/workspace";
import { deleteEntries, renameEntry } from "../../lib/fileTreeOps";
import { useConnectionStore } from "../../stores/connectionStore";
import { useEditorStore } from "../../stores/editorStore";
import { useWorkspaceChangeStore } from "../../stores/workspaceChangeStore";
import { GalleryPanel } from "./GalleryPanel";

vi.mock("../../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/workspace")>();
  return {
    ...actual,
    fetchMedia: vi.fn(),
    fetchMediaThumb: vi.fn(() => new Promise<Blob>(() => {})),
    readBytes: vi.fn(),
    openWithDefaultApp: vi.fn(),
    revealInFileManager: vi.fn(),
  };
});

// Trashing goes through the explorer's delete flow; its confirm dialog and tree
// bookkeeping are not this panel's business.
vi.mock("../../lib/fileTreeOps", () => ({
  deleteEntries: vi.fn(),
  renameEntry: vi.fn(),
}));

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
  vi.restoreAllMocks();
});

beforeEach(async () => {
  await i18n.changeLanguage("en");
  // The view style is persisted; each test starts from the default.
  localStorage.removeItem("litecode.gallery.view");
  vi.mocked(fetchMedia).mockReset();
  vi.mocked(fetchMedia).mockResolvedValue({ entries, truncated: false });
  useWorkspaceChangeStore.setState({ last: null, headSeq: 0 });
  useConnectionStore.setState({ state: "connected", hostOs: "windows" });
  vi.mocked(revealInFileManager).mockReset();
  vi.mocked(revealInFileManager).mockResolvedValue(undefined);
  vi.mocked(deleteEntries).mockReset();
  vi.mocked(deleteEntries).mockResolvedValue(true);
  vi.mocked(renameEntry).mockReset();
  vi.mocked(renameEntry).mockResolvedValue("art/cover.png");
  vi.mocked(openWithDefaultApp).mockReset();
  vi.mocked(openWithDefaultApp).mockResolvedValue(undefined);
  vi.mocked(readBytes).mockReset();
  // jsdom ships no object-URL factory and no media playback.
  (URL as unknown as { createObjectURL: unknown }).createObjectURL = vi.fn(
    () => "blob:gallery",
  );
  (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = vi.fn();
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
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

  it("re-reads after a workspace change without blanking the grid", async () => {
    renderPanel();
    await screen.findByText("one.png");
    expect(fetchMedia).toHaveBeenCalledTimes(1);

    act(() => {
      useWorkspaceChangeStore.getState().record(["art/one.png"], "modified");
    });

    await waitFor(
      () => expect(fetchMedia).toHaveBeenCalledTimes(2),
      { timeout: 2000 },
    );
    // Rely on the server snapshot: it already knows the change.
    expect(vi.mocked(fetchMedia).mock.calls[1]?.[0]).toEqual({ fresh: false });
    // The old grid stays on screen while the reload runs.
    expect(screen.getByText("one.png")).toBeTruthy();
  });

  it("asks for a walk when the connection resumes", async () => {
    useConnectionStore.setState({ state: "disconnected" });
    renderPanel();
    await screen.findByText("one.png");

    act(() => {
      useConnectionStore.setState({ state: "connected" });
    });

    await waitFor(() => expect(fetchMedia).toHaveBeenCalledTimes(2));
    // Frames may have been missed while the socket was down.
    expect(vi.mocked(fetchMedia).mock.calls[1]?.[0]).toEqual({ fresh: true });
  });

  it("refreshes with a forced walk", async () => {
    renderPanel();
    await screen.findByText("one.png");

    fireEvent.click(screen.getByRole("button", { name: "Refresh media" }));

    await waitFor(() => expect(fetchMedia).toHaveBeenCalledTimes(2));
    expect(vi.mocked(fetchMedia).mock.calls[1]?.[0]).toEqual({ fresh: true });
  });

  it("opens previewable media in its own editor tab", async () => {
    const openFile = vi.fn();
    useEditorStore.setState({ openFile } as never);
    renderPanel();
    const tile = await screen.findByRole("button", { name: "one.png" });

    fireEvent.doubleClick(tile);

    expect(openFile).toHaveBeenCalledWith("art/one.png");
    expect(openWithDefaultApp).not.toHaveBeenCalled();
  });

  it("hands a container the panel cannot decode to the system player", async () => {
    vi.mocked(fetchMedia).mockResolvedValue({
      entries: [
        {
          path: "clips/reel.mkv",
          name: "reel.mkv",
          folder: "clips",
          kind: "video",
          size: 900,
          mtime: 5,
        },
      ],
      truncated: false,
    });
    const openFile = vi.fn();
    useEditorStore.setState({ openFile } as never);
    renderPanel();
    const tile = await screen.findByRole("button", { name: "reel.mkv" });

    fireEvent.doubleClick(tile);

    expect(openWithDefaultApp).toHaveBeenCalledWith("clips/reel.mkv");
    expect(openFile).not.toHaveBeenCalled();
  });

  it("does not hover-preview a container it cannot decode", async () => {
    vi.mocked(fetchMedia).mockResolvedValue({
      entries: [
        {
          path: "clips/reel.mkv",
          name: "reel.mkv",
          folder: "clips",
          kind: "video",
          size: 900,
          mtime: 5,
        },
      ],
      truncated: false,
    });
    renderPanel();
    const tile = await screen.findByRole("button", { name: "reel.mkv" });

    fireEvent.mouseEnter(tile);

    // Longer than the open delay: no card must appear, and no bytes are read.
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(screen.queryByTestId("gallery-hover")).toBeNull();
    expect(readBytes).not.toHaveBeenCalled();
  });

  it("keeps the selection footer informational", async () => {
    const { container } = renderPanel();
    await screen.findByText("one.png");

    fireEvent.click(screen.getByRole("button", { name: "one.png" }));

    expect(container.querySelector(".gallery-detail")).toBeTruthy();
    expect(container.querySelectorAll(".gallery-detail button").length).toBe(0);
  });

  it("loops a video in a hover card", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob(["x"], { type: "video/mp4" }),
    );
    renderPanel();
    const tile = await screen.findByRole("button", { name: "two.mp4" });

    fireEvent.mouseEnter(tile);

    const card = await screen.findByTestId("gallery-hover", undefined, {
      timeout: 3000,
    });
    // The card paints first and swaps in the player once the bytes land.
    const video = await waitFor(() => {
      const node = card.querySelector("video");
      expect(node).toBeTruthy();
      return node;
    });
    expect(video?.loop).toBe(true);
    expect(readBytes).toHaveBeenCalledWith("clips/two.mp4");

    fireEvent.mouseLeave(tile);
    await waitFor(() => expect(screen.queryByTestId("gallery-hover")).toBeNull(), {
      timeout: 3000,
    });
  });

  it("keeps the hover card when another panel scrolls", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob(["x"], { type: "video/mp4" }),
    );
    renderPanel();
    const tile = await screen.findByRole("button", { name: "two.mp4" });

    fireEvent.mouseEnter(tile);
    await screen.findByTestId("gallery-hover", undefined, { timeout: 3000 });

    // A transcript gliding as tokens arrive scrolls its own scroller; the tile
    // has not moved, so the card keeps playing.
    const transcript = document.createElement("div");
    document.body.appendChild(transcript);
    transcript.dispatchEvent(new Event("scroll"));
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(screen.queryByTestId("gallery-hover")).not.toBeNull();
    transcript.remove();

    // A scroller holding the tile does move it, and the card closes.
    document.dispatchEvent(new Event("scroll"));
    await waitFor(() => expect(screen.queryByTestId("gallery-hover")).toBeNull(), {
      timeout: 3000,
    });
  });
  it("folds each type until it is opened", async () => {
    renderPanel();
    await screen.findByText("one.png");
    fireEvent.click(screen.getByRole("tab", { name: "Types" }));

    const images = screen.getByRole("button", { name: /Images/ });
    expect(images.getAttribute("aria-expanded")).toBe("false");
    // Folded sections mount no tiles at all.
    expect(screen.queryByRole("button", { name: "one.png" })).toBeNull();

    fireEvent.click(images);
    expect(images.getAttribute("aria-expanded")).toBe("true");
    expect(await screen.findByRole("button", { name: "one.png" })).toBeTruthy();

    fireEvent.click(images);
    expect(screen.queryByRole("button", { name: "one.png" })).toBeNull();
  });

  it("offers reveal and trashing from a tile's context menu", async () => {
    renderPanel();
    const tile = await screen.findByRole("button", { name: "one.png" });

    fireEvent.contextMenu(tile);
    const reveal = await screen.findByRole("menuitem", {
      name: "Reveal in File Explorer",
    });
    expect(screen.getByRole("menuitem", { name: "Open" })).toBeTruthy();

    fireEvent.click(reveal);
    expect(revealInFileManager).toHaveBeenCalledWith("art/one.png");
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.contextMenu(tile);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete" }));
    expect(deleteEntries).toHaveBeenCalledWith(["art/one.png"]);
    // The tile leaves at once instead of waiting for the listing round-trip.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "one.png" })).toBeNull(),
    );
  });

  it("renames a tile in place from its context menu", async () => {
    renderPanel();
    const tile = await screen.findByRole("button", { name: "one.png" });

    fireEvent.contextMenu(tile);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }));

    const input = screen.getByDisplayValue("one.png") as HTMLInputElement;
    // The stem is preselected, so typing keeps the extension.
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 3]);

    fireEvent.change(input, { target: { value: "cover.png" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(renameEntry).toHaveBeenCalledWith("art/one.png", "cover.png"),
    );
    // The editor closes and the caption follows the rename in the same beat.
    expect(screen.queryByDisplayValue("cover.png")).toBeNull();
    expect(await screen.findByRole("button", { name: "cover.png" })).toBeTruthy();
  });

  it("switches the whole panel between thumbnails and list", async () => {
    const { container } = renderPanel();
    await screen.findByText("one.png");
    expect(container.querySelector(".gallery-grid")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "one.png" }).className,
    ).toContain("gallery-tile");

    fireEvent.click(screen.getByRole("button", { name: "List" }));

    expect(container.querySelector(".gallery-list")).toBeTruthy();
    expect(container.querySelector(".gallery-grid")).toBeNull();
    expect(
      screen.getByRole("button", { name: "one.png" }).className,
    ).toContain("gallery-row");
    // The choice is remembered for the next panel mount.
    expect(localStorage.getItem("litecode.gallery.view")).toBe("list");

    fireEvent.click(screen.getByRole("button", { name: "Thumbnails" }));

    expect(container.querySelector(".gallery-grid")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "one.png" }).className,
    ).toContain("gallery-tile");
  });

  it("shows type, size and modified columns in list view", async () => {
    renderPanel();
    await screen.findByText("one.png");
    fireEvent.click(screen.getByRole("button", { name: "List" }));

    expect(screen.getByText("Image")).toBeTruthy();
    expect(screen.getByText("Video")).toBeTruthy();
    expect(screen.getByText("1.2 KB")).toBeTruthy();
    expect(screen.getByText("2.1 KB")).toBeTruthy();
    // The compact stamp keeps the full timestamp in its tooltip.
    const row = screen.getByRole("button", { name: "one.png" });
    expect(row.querySelector(".gallery-row-time")?.getAttribute("title")).toBeTruthy();
  });

  it("shows folders as tiles in thumbnail view and rows in list view", async () => {
    renderPanel();
    await screen.findByText("one.png");
    fireEvent.click(screen.getByRole("tab", { name: "Folders" }));

    const folder = await screen.findByTitle("art");
    expect(folder.className).toContain("gallery-folder-tile");
    expect(screen.queryByText("one.png")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "List" }));

    expect(screen.getByTitle("art").className).toContain("gallery-row");
    expect(screen.getByTitle("art").className).not.toContain("gallery-folder-tile");
  });

  it("explains an empty workspace", async () => {
    vi.mocked(fetchMedia).mockResolvedValue({ entries: [], truncated: false });
    renderPanel();
    expect(
      await screen.findByText("No media files in this workspace."),
    ).toBeTruthy();
  });
});
