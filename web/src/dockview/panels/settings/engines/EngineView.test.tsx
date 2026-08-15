import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EngineView } from "./EngineView";
import type { EnginesDetail } from "../../../../api/workspace";
import * as workspace from "../../../../api/workspace";

const installServer = vi.spyOn(workspace, "installServer");
const getInstallStatus = vi.spyOn(workspace, "getInstallStatus");
const probeLspServers = vi.spyOn(workspace, "probeLspServers");

function detailFixture(): EnginesDetail {
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
      index: {
        status: "absent",
        exists: false,
        needs_rebuild: false,
        vectors_ready: false,
        indexed_files: 0,
        indexed_chunks: 0,
      },
      policy: {
        product_internal_dirs: [],
        exclude_globs: [],
        extensions: [],
        max_file_bytes: 0,
        binary_files: false,
        lockfiles: false,
        minified_files: false,
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
    },
  };
}

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
  });

  it("stops polling after unmount (no setState on unmounted view)", async () => {
    const user = userEvent.setup();
    installServer.mockResolvedValue({
      task_id: "task-1",
      server_id: "typescript",
      status: "installing",
      progress: { downloaded_bytes: 1, total_bytes: 10 },
    });
    // First poll returns installing again, so the loop would continue; after
    // unmount the guard must prevent the next getInstallStatus call.
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

    // Poll loop is blocked on its 800ms sleep; advancing timers would call
    // getInstallStatus again unless the unmount guard broke the loop. Wait past
    // the poll interval and assert no further status poll happened.
    await new Promise((resolve) => setTimeout(resolve, 900));
    expect(getInstallStatus.mock.calls.length).toBe(callsBeforeUnmount);
  });
});
