import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DesktopComponentProgress, LitecodeDesktopBridge } from "../../../../api/auth";
import {
  EmbedModelDownloadCard,
  cleanIpcError,
  embedDownloadAvailable,
  useEmbedModelDownload,
} from "./EmbedModelDownload";

function Harness({ onInstalled }: { onInstalled: () => void }) {
  const { available, state, start } = useEmbedModelDownload(onInstalled);
  if (!available) return <p>unavailable</p>;
  return <EmbedModelDownloadCard state={state} onStart={() => void start()} />;
}

function installBridge(overrides: Partial<LitecodeDesktopBridge>) {
  let progress: ((event: DesktopComponentProgress) => void) | null = null;
  window.litecode = {
    getSessionMode: () => "local",
    getComponentStatus: vi.fn(async () => ({
      id: "embed-model",
      installed: false,
      path: null,
      downloading: false,
      error: null,
    })),
    onComponentProgress: (handler) => {
      progress = handler;
      return () => {
        progress = null;
      };
    },
    ...overrides,
  };
  return { emit: (event: DesktopComponentProgress) => progress?.(event) };
}

afterEach(() => {
  cleanup();
  delete window.litecode;
});

describe("EmbedModelDownload", () => {
  it("is hidden without the desktop bridge or in remote sessions", () => {
    expect(embedDownloadAvailable()).toBe(false);
    window.litecode = { ensureComponent: vi.fn(), getSessionMode: () => "remote" };
    expect(embedDownloadAvailable()).toBe(false);
  });

  it("downloads with progress, then reports installed", async () => {
    let resolve!: () => void;
    const ensureComponent = vi.fn(
      () =>
        new Promise<{ id: string; path: string; downloaded: boolean }>((r) => {
          resolve = () => r({ id: "embed-model", path: "C:/m", downloaded: true });
        }),
    );
    const { emit } = installBridge({ ensureComponent });
    const onInstalled = vi.fn();
    render(<Harness onInstalled={onInstalled} />);

    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    expect(ensureComponent).toHaveBeenCalledWith("embed-model");
    act(() =>
      emit({ id: "embed-model", phase: "downloading", received: 50, total: 200, message: "Downloading — 50 B" }),
    );
    expect(screen.getByText("25%")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();

    await act(async () => resolve());
    expect(onInstalled).toHaveBeenCalledTimes(1);
  });

  it("shows a retryable error without throwing", async () => {
    const ensureComponent = vi
      .fn()
      .mockRejectedValueOnce(
        new Error("Error invoking remote method 'litecode:ensure-component': Error: sha256 mismatch"),
      )
      .mockResolvedValueOnce({ id: "embed-model", path: "C:/m", downloaded: true });
    installBridge({ ensureComponent });
    const onInstalled = vi.fn();
    render(<Harness onInstalled={onInstalled} />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Download" }));
    });
    expect(screen.getByText("sha256 mismatch")).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    });
    expect(ensureComponent).toHaveBeenCalledTimes(2);
    expect(onInstalled).toHaveBeenCalledTimes(1);
  });

  it("settles from host events when the download was started elsewhere", async () => {
    const { emit } = installBridge({ ensureComponent: vi.fn() });
    const onInstalled = vi.fn();
    render(<Harness onInstalled={onInstalled} />);
    await act(async () => undefined);

    act(() => emit({ id: "embed-model", phase: "downloading", received: 1, total: 4, message: "Downloading" }));
    expect(screen.getByText("25%")).toBeTruthy();
    act(() => emit({ id: "embed-model", phase: "error", message: "failed", error: "offline" }));
    expect(screen.getByText("offline")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();

    // Retried from another view and finished: no error line, no progress bar left behind.
    act(() => emit({ id: "embed-model", phase: "resolving", message: "Looking up" }));
    expect(screen.queryByText("offline")).toBeNull();
    act(() => emit({ id: "embed-model", phase: "done", message: "ready" }));
    const card = screen.getByRole("group", { name: "Embedding model download" });
    expect(card.getAttribute("data-phase")).toBe("done");
    expect(screen.queryByText("offline")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
    // Restart + refetch is the app-wide watcher's job; the card does not double it.
    expect(onInstalled).not.toHaveBeenCalled();
  });

  it("restores a host-side error after the view is reopened", async () => {
    installBridge({
      ensureComponent: vi.fn(),
      getComponentStatus: vi.fn(async () => ({
        id: "embed-model",
        installed: false,
        path: null,
        downloading: false,
        error: "Cannot read litecode-components.json",
      })),
    });
    render(<Harness onInstalled={vi.fn()} />);
    expect(await screen.findByText("Cannot read litecode-components.json")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });

  it("cleans Electron IPC error prefixes", () => {
    expect(cleanIpcError(new Error("Error invoking remote method 'x': Error: boom"))).toBe("boom");
    expect(cleanIpcError("plain")).toBe("plain");
  });
});
