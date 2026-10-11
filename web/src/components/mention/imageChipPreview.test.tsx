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
  });

  it("leaves text, symbols, line ranges, and outside paths as capsules", () => {
    expect(imageChipPath("file", { id: "src/a.rs" })).toBeNull();
    expect(imageChipPath("node", { id: "assets/icon.png" })).toBeNull();
    expect(imageChipPath("file", { id: "assets/icon.png", symbol: "Icon" })).toBeNull();
    expect(imageChipPath("file", { id: "assets/icon.png", lines: "1-2" })).toBeNull();
    expect(imageChipPath("file", { id: "C:/shot.png" })).toBeNull();
    expect(imageChipPath("file", { id: "" })).toBeNull();
  });
});

describe("ImageChipPreview", () => {
  it("waits for the pointer to rest, then paints a fixed box", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob([Uint8Array.from([1, 2, 3])], { type: "image/png" }),
    );
    render(
      <ImageChipPreview path="assets/icon.png">
        <button type="button">icon.png</button>
      </ImageChipPreview>,
    );

    fireEvent.mouseEnter(anchor());
    expect(readBytes).not.toHaveBeenCalled();
    expect(document.querySelector(".image-chip-preview")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(180);
    });

    expect(readBytes).toHaveBeenCalledWith("assets/icon.png");
    const panel = document.querySelector(".image-chip-preview") as HTMLElement | null;
    expect(panel).toBeTruthy();
    expect(panel?.style.width).toBe("240px");
    expect(panel?.style.height).toBe("160px");
    expect(panel?.querySelector("img")?.getAttribute("src")).toBe("blob:preview");
    expect(document.querySelector("button")?.textContent).toBe("icon.png");
  });

  it("does not fetch when the pointer leaves before the delay", async () => {
    vi.mocked(readBytes).mockResolvedValue(new Blob([Uint8Array.from([1])]));
    render(
      <ImageChipPreview path="assets/icon.png">
        <button type="button">icon.png</button>
      </ImageChipPreview>,
    );
    fireEvent.mouseEnter(anchor());
    fireEvent.mouseLeave(anchor());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(180);
    });
    expect(readBytes).not.toHaveBeenCalled();
    expect(document.querySelector(".image-chip-preview")).toBeNull();
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
      await vi.advanceTimersByTimeAsync(180);
    });
    expect(document.querySelector(".image-chip-preview-note")?.textContent).toBe(
      "Can't preview",
    );
  });

  it("closes when the pointer leaves", async () => {
    vi.mocked(readBytes).mockResolvedValue(
      new Blob([Uint8Array.from([1])], { type: "image/gif" }),
    );
    render(
      <ImageChipPreview path="shots/wave.gif">
        <button type="button">wave.gif</button>
      </ImageChipPreview>,
    );
    fireEvent.mouseEnter(anchor());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(180);
    });
    expect(document.querySelector(".image-chip-preview")).toBeTruthy();
    fireEvent.mouseLeave(anchor());
    expect(document.querySelector(".image-chip-preview")).toBeNull();
  });
});
