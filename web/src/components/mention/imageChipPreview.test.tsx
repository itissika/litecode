import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { readBytes } from "../../api/workspace";
import { clearImageChipPreviewCache, imageChipPath } from "./imageChipHover";
import { ImageChipPreview } from "./imageChipPreview";

vi.mock("../../api/workspace", () => ({
  readBytes: vi.fn(),
}));

function anchor(): HTMLElement {
  const node = document.querySelector(".image-chip-anchor");
  if (!node) throw new Error("missing anchor");
  return node as HTMLElement;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:preview");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  vi.mocked(readBytes).mockReset();
});

afterEach(() => {
  clearImageChipPreviewCache();
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("imageChipPath", () => {
  it("selects a whole-file png or gif in the workspace", () => {
    expect(imageChipPath("file", { id: "assets/icon.png" })).toBe("assets/icon.png");
    expect(imageChipPath("file", { id: "shots/wave.GIF" })).toBe("shots/wave.GIF");
    expect(imageChipPath("file", { id: "assets/why/layout.webp" })).toBe(
      "assets/why/layout.webp",
    );
    expect(imageChipPath("file", { id: "shots/clip.mp4" })).toBe("shots/clip.mp4");
    expect(imageChipPath("file", { id: "shots/clip.MOV" })).toBe("shots/clip.MOV");
  });

  it("leaves text, symbols, line ranges, and outside paths as capsules", () => {
    expect(imageChipPath("file", { id: "src/a.rs" })).toBeNull();
    expect(imageChipPath("file", { id: "shots/clip.mkv" })).toBeNull();
    expect(imageChipPath("file", { id: "voice.mp3" })).toBeNull();
    expect(imageChipPath("node", { id: "assets/icon.png" })).toBeNull();
    expect(imageChipPath("file", { id: "assets/icon.png", symbol: "Icon" })).toBeNull();
    expect(imageChipPath("file", { id: "assets/icon.png", lines: "1-2" })).toBeNull();
    expect(imageChipPath("file", { id: "C:/shot.png" })).toBeNull();
    expect(imageChipPath("file", { id: "" })).toBeNull();
  });
});

describe("ImageChipPreview", () => {
  it("opens around the picture as soon as the pointer enters", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob([Uint8Array.from([1, 2, 3])], { type: "image/png" }),
    );
    render(
      <ImageChipPreview path="assets/icon.png">
        <button type="button">icon.png</button>
      </ImageChipPreview>,
    );

    fireEvent.mouseEnter(anchor());
    const panel = document.querySelector(".image-chip-preview") as HTMLElement | null;
    expect(panel).toBeTruthy();
    expect(panel?.style.width).toBe("");
    expect(panel?.style.height).toBe("");
    expect(document.querySelector("button")?.textContent).toBe("icon.png");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(readBytes).toHaveBeenCalledWith("assets/icon.png");
    expect(panel?.querySelector("img")?.getAttribute("src")).toBe("blob:preview");
  });

  it("opens the full reference in the same panel, with no wait", () => {
    render(
      <ImageChipPreview text={"src/a.rs : impl Store › fn save\nmissing"}>
        <button type="button">a.rs</button>
      </ImageChipPreview>,
    );

    expect(document.querySelector(".image-chip-preview")).toBeNull();
    fireEvent.mouseEnter(anchor());
    const panel = document.querySelector(".image-chip-preview") as HTMLElement | null;
    expect(panel?.classList.contains("is-text")).toBe(true);
    expect(panel?.textContent).toBe("src/a.rs : impl Store › fn save\nmissing");
    expect(panel?.style.width).toBe("");
    expect(panel?.style.height).toBe("");
    fireEvent.mouseLeave(anchor());
    expect(document.querySelector(".image-chip-preview")).toBeNull();
  });

  it("puts the picture and the reference in one panel", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob([Uint8Array.from([1])], { type: "image/png" }),
    );
    render(
      <ImageChipPreview path="assets/icon.png" text="assets/icon.png">
        <button type="button">icon.png</button>
      </ImageChipPreview>,
    );

    fireEvent.mouseEnter(anchor());
    const panel = document.querySelector(".image-chip-preview") as HTMLElement | null;
    expect(panel?.classList.contains("has-caption")).toBe(true);
    expect(panel?.textContent).toBe("assets/icon.png");
    expect(panel?.style.width).toBe("");
    expect(panel?.style.height).toBe("");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(panel?.querySelector("img")?.getAttribute("src")).toBe("blob:preview");
  });

  it("plays a whole-file video muted in the same panel", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob([Uint8Array.from([1])], { type: "video/mp4" }),
    );
    render(
      <ImageChipPreview path="shots/clip.mp4">
        <button type="button">clip.mp4</button>
      </ImageChipPreview>,
    );
    fireEvent.mouseEnter(anchor());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const video = document.querySelector("video");
    expect(video?.getAttribute("src")).toBe("blob:preview");
    expect(video?.muted).toBe(true);
    expect(video?.loop).toBe(true);
    expect(document.querySelector("img")).toBeNull();
  });

  it("says it cannot preview when the bytes fail", async () => {
    vi.mocked(readBytes).mockRejectedValue(new Error("missing"));
    render(
      <ImageChipPreview path="assets/missing.png">
        <button type="button">missing.png</button>
      </ImageChipPreview>,
    );
    fireEvent.mouseEnter(anchor());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(document.querySelector(".image-chip-preview-note")?.textContent).toBe(
      "Can't preview",
    );
  });

  it("closes when the pointer leaves", () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob([Uint8Array.from([1])], { type: "image/gif" }),
    );
    render(
      <ImageChipPreview path="shots/wave.gif">
        <button type="button">wave.gif</button>
      </ImageChipPreview>,
    );
    fireEvent.mouseEnter(anchor());
    expect(document.querySelector(".image-chip-preview")).toBeTruthy();
    fireEvent.mouseLeave(anchor());
    expect(document.querySelector(".image-chip-preview")).toBeNull();
  });
});
