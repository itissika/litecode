import { afterEach, describe, expect, it, vi } from "vitest";

import { gitLog, gitStatus } from "../api/workspace";
import {
  actionTargetPaths,
  gitRowId,
  isGitMetaPath,
  parseGitRowId,
  selectedPaths,
  useGitStore,
  watchPathsAffectGitWorktree,
} from "./gitStore";

vi.mock("../api/workspace", () => ({
  gitStatus: vi.fn(),
  gitLog: vi.fn(),
  gitStage: vi.fn(),
  gitUnstage: vi.fn(),
  gitRestore: vi.fn(),
  gitCommit: vi.fn(),
  gitPull: vi.fn(),
  gitPush: vi.fn(),
}));

describe("git row ids", () => {
  it("round-trips section and path", () => {
    const id = gitRowId("changes", "src/foo.rs");
    expect(parseGitRowId(id)).toEqual({
      section: "changes",
      path: "src/foo.rs",
    });
  });

  it("filters selected paths by section", () => {
    const selected = new Set([
      gitRowId("staged", "a.ts"),
      gitRowId("changes", "b.ts"),
      gitRowId("changes", "c.ts"),
    ]);
    expect(selectedPaths(selected, "staged")).toEqual(["a.ts"]);
    expect(selectedPaths(selected, "changes")).toEqual(["b.ts", "c.ts"]);
  });
});

describe("git watch path filter", () => {
  it("treats .git internals as metadata", () => {
    expect(isGitMetaPath(".git/index")).toBe(true);
    expect(isGitMetaPath(".git/HEAD")).toBe(true);
    expect(isGitMetaPath("nested/.git/config")).toBe(true);
    expect(isGitMetaPath("src/main.rs")).toBe(false);
  });

  it("ignores refresh when every changed path is git metadata", () => {
    expect(watchPathsAffectGitWorktree([".git/index", ".git/HEAD"])).toBe(
      false,
    );
    expect(watchPathsAffectGitWorktree([".git/index", "src/a.ts"])).toBe(true);
    expect(watchPathsAffectGitWorktree([])).toBe(true);
  });
});

describe("git file action targets", () => {
  it("applies file actions to the multi-selection when the click target is selected", () => {
    const selected = new Set([
      gitRowId("changes", "a.ts"),
      gitRowId("changes", "b.ts"),
    ]);
    expect(actionTargetPaths(selected, "changes", "a.ts")).toEqual([
      "a.ts",
      "b.ts",
    ]);
    expect(actionTargetPaths(selected, "changes", "c.ts")).toEqual(["c.ts"]);
    expect(
      actionTargetPaths(
        new Set([gitRowId("changes", "a.ts")]),
        "changes",
        "a.ts",
      ),
    ).toEqual(["a.ts"]);
  });
});

const repoStatus = {
  is_repo: true,
  branch: "main",
  upstream_ahead: 0,
  upstream_behind: 0,
  staged: [],
  changes: [],
};

describe("git refresh timing", () => {
  const status = vi.mocked(gitStatus);
  const log = vi.mocked(gitLog);

  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
    status.mockReset();
    log.mockReset();
  });

  it("refreshes status while the panel is hidden and skips the log for a worktree edit", async () => {
    vi.useFakeTimers();
    status.mockResolvedValue(repoStatus);
    log.mockResolvedValue({ is_repo: true, commits: [] });
    useGitStore.setState({ visible: false });
    useGitStore.getState().scheduleRefresh(["src/a.ts"]);
    await vi.advanceTimersByTimeAsync(400);
    expect(status).toHaveBeenCalledTimes(1);
    expect(log).not.toHaveBeenCalled();
  });

  it("keeps a history refresh when a file edit lands in the same quiet period", async () => {
    vi.useFakeTimers();
    status.mockResolvedValue(repoStatus);
    log.mockResolvedValue({ is_repo: true, commits: [] });
    useGitStore.setState({ visible: false });
    useGitStore.getState().scheduleRefresh(undefined, { history: true });
    useGitStore.getState().scheduleRefresh(["src/a.ts"]);
    await vi.advanceTimersByTimeAsync(400);
    expect(status).toHaveBeenCalledTimes(1);
    expect(log).toHaveBeenCalledTimes(1);
  });

  it("does not refresh for a .git path on the worktree bus", async () => {
    vi.useFakeTimers();
    status.mockResolvedValue(repoStatus);
    useGitStore.getState().scheduleRefresh([".git/index"]);
    await vi.advanceTimersByTimeAsync(400);
    expect(status).not.toHaveBeenCalled();
    expect(log).not.toHaveBeenCalled();
  });

  it("runs a queued history refresh after an in-flight status refresh", async () => {
    vi.useFakeTimers();
    let release: (value: typeof repoStatus) => void = () => {};
    status.mockResolvedValue(repoStatus);
    status.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    log.mockResolvedValue({ is_repo: true, commits: [] });
    const first = useGitStore.getState().refresh({ history: false });
    useGitStore.getState().scheduleRefresh(undefined, { history: true });
    await vi.advanceTimersByTimeAsync(400);
    expect(log).not.toHaveBeenCalled();
    release(repoStatus);
    await first;
    expect(log).toHaveBeenCalledTimes(1);
  });
});
