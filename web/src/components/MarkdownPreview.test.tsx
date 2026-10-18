import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { fileMentionSource, symbolMentionSource } from "../lib/knowledge/markers";
import { useEditorStore } from "../stores/editorStore";
import { AgentMarkdown } from "./AgentMarkdown";
import { loadImageChipPreview } from "./mention/imageChipHover";

vi.mock("./mention/imageChipHover", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./mention/imageChipHover")>();
  return {
    ...actual,
    loadImageChipPreview: vi.fn(async (path: string) => `blob:${path}`),
  };
});

afterEach(() => {
  cleanup();
  vi.mocked(loadImageChipPreview).mockClear();
});

const preview = { basePath: "docs/readme.md" };

describe("markdown file preview images", () => {
  it("loads a relative image from the workspace and leaves a remote image alone", async () => {
    const { container } = render(
      <AgentMarkdown
        preview={preview}
        text={"![wave](./shot.gif)\n\n![remote](https://example.com/a.gif)"}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("img", { name: "wave" }).getAttribute("src")).toBe(
        "blob:docs/shot.gif",
      );
    });
    expect(loadImageChipPreview).toHaveBeenCalledWith("docs/shot.gif");
    const remote = screen.getByRole("img", { name: "remote" });
    expect(remote.getAttribute("src")).toBe("https://example.com/a.gif");
    expect(container.querySelector("img[src='https://example.com/a.gif']")).toBeTruthy();
    expect(loadImageChipPreview).toHaveBeenCalledTimes(1);
  });

  it("does not load a path that escapes the workspace or is not an image", async () => {
    render(
      <AgentMarkdown
        preview={{ basePath: "readme.md" }}
        text={"![secret](../secret.gif)\n\n![notes](./notes.md)"}
      />,
    );
    await waitFor(() => {
      expect(screen.getByText("secret")).toBeTruthy();
    });
    expect(screen.getByText("notes")).toBeTruthy();
    expect(screen.queryByRole("img")).toBeNull();
    expect(loadImageChipPreview).not.toHaveBeenCalled();
  });

  it("draws a whole-file image chip and keeps symbol and line chips as text", async () => {
    const openFile = vi.fn(async () => {});
    useEditorStore.setState({ openFile } as never);
    const symbol = symbolMentionSource("docs/shot.gif", { symbol: "Icon" });
    const lines = symbolMentionSource("docs/shot.gif", { lines: "1-2" });
    render(
      <AgentMarkdown
        preview={preview}
        text={`${fileMentionSource("docs/shot.gif")} ${symbol} ${lines}`}
      />,
    );

    const chip = screen.getByRole("button", { name: "shot.gif" });
    expect(screen.getByRole("button", { name: "shot.gif : Icon" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "shot.gif : 1-2" })).toBeTruthy();
    await waitFor(() => {
      expect(screen.getByRole("img", { name: "shot.gif" })).toBeTruthy();
    });
    expect(screen.getAllByRole("img")).toHaveLength(1);
    // The picture is a button too; the chip comes first.
    expect(screen.getAllByRole("button", { name: "shot.gif" })).toHaveLength(2);
    fireEvent.click(chip);
    expect(openFile).toHaveBeenCalledWith("docs/shot.gif");
    expect(loadImageChipPreview).toHaveBeenCalledTimes(1);
    expect(loadImageChipPreview).toHaveBeenCalledWith("docs/shot.gif");
  });

  it("keeps an ordinary markdown link as a link", () => {
    render(
      <AgentMarkdown preview={preview} text={"[guide](./guide.md)"} />,
    );
    const link = screen.getByRole("link", { name: "guide" });
    expect(link.getAttribute("href")).toBe("./guide.md");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("does not load image bytes when preview is off", async () => {
    const { container } = render(
      <AgentMarkdown
        text={`${fileMentionSource("docs/shot.gif")}\n\n![wave](./shot.gif)`}
      />,
    );
    expect(container.textContent).toContain('[@ file="docs/shot.gif"]');
    expect(screen.queryByRole("button")).toBeNull();
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe("./shot.gif");
    await waitFor(() => {
      expect(loadImageChipPreview).not.toHaveBeenCalled();
    });
  });
});
