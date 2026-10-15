import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { resetCentreSettleForTests, settleCentre } from "../lib/centreSettle";
import { useTreeStore } from "../stores/treeStore";
import { FileTree } from "./FileTree";

const originalLoadRoot = useTreeStore.getState().loadRoot;
let loadRoot: ReturnType<typeof vi.fn>;

const flush = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

beforeEach(() => {
  resetCentreSettleForTests();
  loadRoot = vi.fn(async () => {});
  useTreeStore.setState({ loadRoot: loadRoot as never });
});

afterEach(() => {
  cleanup();
  useTreeStore.setState({ loadRoot: originalLoadRoot });
  resetCentreSettleForTests();
});

describe("WP-170 FileTree root load waits for the centre", () => {
  it("does not load the root before the centre settles, then loads once", async () => {
    render(<FileTree />);
    await flush();
    expect(loadRoot).not.toHaveBeenCalled();
    settleCentre();
    await flush();
    expect(loadRoot).toHaveBeenCalledTimes(1);
  });

  it("does not load after unmount", async () => {
    const { unmount } = render(<FileTree />);
    unmount();
    settleCentre();
    await flush();
    expect(loadRoot).not.toHaveBeenCalled();
  });
});
