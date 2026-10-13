import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { EngineView } from "./EngineView";
import type { EnginesDetail } from "../../../../api/workspace";
import type { WorkspaceEnginesDoc } from "../../../../api/settings";
import * as workspace from "../../../../api/workspace";
import { useSettingsStore } from "../../../../stores/settingsStore";

const installServer = vi.spyOn(workspace, "installServer");
const getInstallStatus = vi.spyOn(workspace, "getInstallStatus");
const probeLspServers = vi.spyOn(workspace, "probeLspServers");
const saveEngines = vi.fn(async (_file: WorkspaceEnginesDoc) => undefined);

function detailFixture(patch?: Partial<EnginesDetail["lsp"]>): EnginesDetail {
  return {
    retrieval: {
      desired: false,
      state: "stopped",
      usable: "stopped",
      error: null,
      model: {
        model_dir: "/models",
        model_found: false,
        tokenizer_found: false,
        ready: false,
      },
      total: { state: "ready", phase: null, chunks_done: 0, chunks_total: 0, chunks_pending: 0, files_pending: 0, error: null },
      code: { state: "ready", phase: null, chunks_done: 0, chunks_total: 0, chunks_pending: 0, files_pending: 0, error: null },
      session: { state: "ready", phase: null, chunks_done: 0, chunks_total: 0, chunks_pending: 0, files_pending: 0, error: null },
      index: {
        exists: false,
        needs_rebuild: false,
        vectors_ready: false,
        indexed_files: 0,
      },
      policy: {
        product_internal_dirs: [],
        exclude_globs: [],
        max_file_bytes: 0,
        binary_files: false,
      },
    },
    lsp: {
      desired: false,
      usable: "stopped",
      configured_servers: [],
      error: null,
      probes: [
        {
          id: "typescript",
          command: "tsc",
          sources: ["npx"],
          status: "missing",
        },
      ],
      ...patch,
    },
  };
}

const availableProbe = {
  id: "typescript",
  command: "tsc",
  sources: ["npx"],
  status: "available" as const,
};

beforeEach(() => {
  saveEngines.mockReset().mockImplementation(async (file) => {
    useSettingsStore.setState({ engines: file });
  });
  useSettingsStore.setState({
    persistByDoc: {},
    engines: {
      version: 1,
      lsp: { desired: false, servers: [] },
      retrieval: { desired: false },
    },
    saveEngines,
  });
  probeLspServers.mockReset().mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("EngineView LSP install poll", () => {
  it("renders retrieval policy tip without crashing on current API shape", () => {
    probeLspServers.mockResolvedValue([]);
    render(<EngineView detail={detailFixture()} onChanged={() => {}} />);
    expect(screen.getByText("Semantic retrieval")).toBeTruthy();
    expect(screen.getByLabelText("No skip paths")).toBeTruthy();
    expect(screen.queryByText("cuda")).toBeNull();
  });

  it("shows a cuda tag only when the worker reports cuda-ort", () => {
    probeLspServers.mockResolvedValue([]);
    const detail = detailFixture();
    detail.retrieval.embed_device = "cpu-ort";
    const { rerender } = render(
      <EngineView detail={detail} onChanged={() => {}} />,
    );
    expect(screen.queryByText("cuda")).toBeNull();
    rerender(
      <EngineView
        detail={{
          ...detail,
          retrieval: { ...detail.retrieval, embed_device: "cuda-ort" },
        }}
        onChanged={() => {}}
      />,
    );
    expect(screen.getByText("cuda")).toBeTruthy();
  });

  it("stops polling after unmount (no setState on unmounted view)", async () => {
    const user = userEvent.setup();
    installServer.mockResolvedValue({
      task_id: "task-1",
      server_id: "typescript",
      status: "installing",
      progress: { downloaded_bytes: 1, total_bytes: 10 },
    });
    getInstallStatus.mockResolvedValue({
      task_id: "task-1",
      server_id: "typescript",
      status: "installing",
      progress: { downloaded_bytes: 2, total_bytes: 10 },
    });
    probeLspServers.mockResolvedValue([
      {
        id: "typescript",
        command: "tsc",
        sources: ["npx"],
        status: "missing",
      },
    ]);

    const { unmount } = render(
      <EngineView detail={detailFixture()} onChanged={() => {}} />,
    );

    await user.click(await screen.findByRole("button", { name: /install/i }));
    await vi.waitFor(() => {
      expect(installServer).toHaveBeenCalledTimes(1);
      expect(getInstallStatus).toHaveBeenCalled();
    });

    const callsBeforeUnmount = getInstallStatus.mock.calls.length;
    unmount();

    await new Promise((resolve) => setTimeout(resolve, 900));
    expect(getInstallStatus.mock.calls.length).toBe(callsBeforeUnmount);
  });
});

describe("EngineView LSP persist vs stale detail", () => {
  it("does not clear servers when persist finishes before engines/detail refreshes", async () => {
    probeLspServers.mockResolvedValue([availableProbe]);
    const detail = detailFixture({
      probes: [availableProbe],
      configured_servers: [],
    });
    render(<EngineView detail={detail} onChanged={() => {}} />);

    fireEvent.click(
      await screen.findByRole("button", {
        name: /typescript server, disabled/i,
      }),
    );

    await waitFor(() => {
      expect(saveEngines).toHaveBeenCalledWith(
        expect.objectContaining({
          lsp: expect.objectContaining({ servers: ["typescript"] }),
        }),
      );
    });

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(saveEngines).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("button", { name: /typescript server, enabled/i }),
    ).toBeTruthy();
  });

  it("does not ping-pong writes across overlapping detail snapshots", async () => {
    probeLspServers.mockResolvedValue([availableProbe]);
    const empty = detailFixture({
      probes: [availableProbe],
      configured_servers: [],
      usable: "stopped",
    });
    const enabled = detailFixture({
      probes: [availableProbe],
      configured_servers: ["typescript"],
      desired: true,
      usable: "warming",
    });

    const { rerender } = render(
      <EngineView detail={empty} onChanged={() => {}} />,
    );

    fireEvent.click(
      await screen.findByRole("button", {
        name: /typescript server, disabled/i,
      }),
    );
    await waitFor(() => {
      expect(saveEngines).toHaveBeenCalledTimes(1);
    });

    rerender(<EngineView detail={enabled} onChanged={() => {}} />);
    rerender(<EngineView detail={empty} onChanged={() => {}} />);
    rerender(<EngineView detail={enabled} onChanged={() => {}} />);
    await new Promise((resolve) => setTimeout(resolve, 50));

    expect(saveEngines).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("button", { name: /typescript server, enabled/i }),
    ).toBeTruthy();
  });

  it("shows stop while warming if desired is on, and play/stop writes engines not usable", async () => {
    probeLspServers.mockResolvedValue([availableProbe]);
    useSettingsStore.setState({
      engines: {
        version: 1,
        lsp: { desired: true, servers: ["typescript"] },
        retrieval: { desired: false },
      },
    });
    render(
      <EngineView
        detail={detailFixture({
          probes: [availableProbe],
          configured_servers: ["typescript"],
          desired: true,
          usable: "warming",
        })}
        onChanged={() => {}}
      />,
    );
    const lsp = screen.getByText("Language servers").closest("section");
    expect(lsp).toBeTruthy();
    const stop = within(lsp as HTMLElement).getByRole("button", {
      name: "Stop engine",
    });
    expect(stop).toBeTruthy();
    expect((stop as HTMLButtonElement).disabled).toBe(false);

    fireEvent.click(stop);
    await waitFor(() => {
      expect(saveEngines).toHaveBeenCalledWith(
        expect.objectContaining({
          lsp: { desired: false, servers: ["typescript"] },
        }),
      );
    });
    fireEvent.click(
      within(lsp as HTMLElement).getByRole("button", { name: "Start engine" }),
    );
    await waitFor(() => {
      expect(saveEngines).toHaveBeenCalledTimes(2);
      expect(saveEngines).toHaveBeenLastCalledWith(
        expect.objectContaining({
          lsp: { desired: true, servers: ["typescript"] },
        }),
      );
    });
  });
});

describe("EngineView embed model (slim installer)", () => {
  const MISSING = "embed model not installed: download it in Settings → Engines";

  function missingModel(): EnginesDetail {
    const detail = detailFixture();
    detail.retrieval = {
      ...detail.retrieval,
      desired: true,
      state: "failed",
      usable: "unavailable",
      error: MISSING,
      model: { model_found: false, tokenizer_found: false, ready: false },
    };
    return detail;
  }

  function ready(): EnginesDetail {
    const detail = detailFixture();
    detail.retrieval = {
      ...detail.retrieval,
      desired: true,
      state: "warm",
      usable: "ready",
      error: null,
      model: {
        model_dir: "/bundles/models/ibm-granite/granite-embedding-97m-multilingual-r2",
        model_found: true,
        tokenizer_found: true,
        ready: true,
      },
    };
    return detail;
  }

  beforeEach(() => {
    useSettingsStore.setState({
      engines: { version: 1, lsp: { desired: false, servers: [] }, retrieval: { desired: true } },
    });
  });

  afterEach(() => {
    delete window.litecode;
  });

  function section(): HTMLElement {
    return screen.getByText("Semantic retrieval").closest("section") as HTMLElement;
  }

  it("lets the download card own the missing-model state, then goes fully green", () => {
    window.litecode = {
      getSessionMode: () => "local",
      ensureComponent: vi.fn(),
      getComponentStatus: vi.fn(async () => ({
        id: "embed-model",
        installed: false,
        path: null,
        downloading: false,
        error: null,
      })),
      onComponentProgress: () => () => undefined,
    };
    const { rerender } = render(<EngineView detail={missingModel()} onChanged={() => {}} />);
    const retrieval = within(section());
    expect(retrieval.getByRole("group", { name: "Embedding model download" })).toBeTruthy();
    expect(retrieval.getByText("needs model")).toBeTruthy();
    expect(retrieval.queryByText("unavailable")).toBeNull();
    expect(retrieval.queryByText(MISSING)).toBeNull();
    expect(retrieval.queryByText(/not found:/)).toBeNull();

    rerender(<EngineView detail={ready()} onChanged={() => {}} />);
    const after = within(section());
    expect(after.getByText("ready")).toBeTruthy();
    expect(after.queryByRole("group", { name: "Embedding model download" })).toBeNull();
    expect(after.queryByText("needs model")).toBeNull();
    expect(after.queryByText(MISSING)).toBeNull();
    expect(after.queryByText(/not found:/)).toBeNull();
    expect(section().querySelector(".text-\\(--_dk-red-500\\)")).toBeNull();
  });

  it("keeps the plain error + not-found line where no download is offered (browser / remote)", () => {
    render(<EngineView detail={missingModel()} onChanged={() => {}} />);
    const retrieval = within(section());
    expect(retrieval.queryByRole("group", { name: "Embedding model download" })).toBeNull();
    expect(retrieval.getByText("unavailable")).toBeTruthy();
    expect(retrieval.getByText(MISSING)).toBeTruthy();
    expect(retrieval.getByText(/not found:/)).toBeTruthy();
  });

  it("after a successful download restarts the failed engine and refetches detail", async () => {
    const refresh = vi.spyOn(workspace, "refreshRetrieval").mockResolvedValue({ desired: true, mode: "full" } as never);
    const detailSpy = vi.spyOn(workspace, "getEnginesDetail").mockResolvedValue(missingModel());
    window.litecode = {
      getSessionMode: () => "local",
      ensureComponent: vi.fn(async () => ({ id: "embed-model", path: "C:/m", downloaded: true })),
      getComponentStatus: vi.fn(async () => ({
        id: "embed-model",
        installed: false,
        path: null,
        downloading: false,
        error: null,
      })),
      onComponentProgress: () => () => undefined,
    };
    const onChanged = vi.fn();
    render(<EngineView detail={missingModel()} onChanged={onChanged} />);
    fireEvent.click(within(section()).getByRole("button", { name: "Download" }));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(refresh).toHaveBeenCalledTimes(1);
    refresh.mockRestore();
    detailSpy.mockRestore();
  });
});
