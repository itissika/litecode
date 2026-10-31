import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DockviewApi } from "dockview-react";

vi.mock("../../lib/centreChat", () => ({ noteLayoutSettled: vi.fn() }));
vi.mock("../config/popoutRestore", () => ({
  applyStagedPopoutBounds: vi.fn(),
  preparePopoutRestore: vi.fn((layout: unknown) => ({ layout, bounds: new Map() })),
  stagePopoutBounds: vi.fn(),
}));
vi.mock("../drag/bindTabDrag", () => ({ bindTabDrag: () => () => {} }));
vi.mock("../popout/orphanOverlay", () => ({ releaseOrphanRenderOverlays: vi.fn() }));
vi.mock("../popout/popoutChrome", () => ({ bindPopoutWindows: () => () => {} }));
vi.mock("./activity", () => ({ bindWindowActivity: () => () => {} }));
vi.mock("./edges", () => ({ recoverDefaultLayout: vi.fn() }));
vi.mock("./events", () => ({ emitPanelRemoved: vi.fn(), onPanelRemoved: () => () => {} }));
vi.mock("./host", () => ({ bindDockview: vi.fn(), dockview: () => null }));
vi.mock("./restoreGate", () => ({ setLayoutFromJsonPending: vi.fn() }));
vi.mock("./keys", () => ({ bindWorkbenchKeys: () => () => {} }));
vi.mock("./placement", () => ({ noteActiveGroup: vi.fn() }));
vi.mock("./windows", () => ({
  bindWindowRegistry: () => () => {},
  onDidRegisterWindow: () => ({ dispose: () => {} }),
  registeredDocuments: () => [],
}));

import { recoverDefaultLayout } from "./edges";
import { LAYOUT_SCHEMA_VERSION, startWorkbench, stopWorkbench } from "./lifecycle";

const STORAGE_KEY = "litecode-dockview-layout-v2";

type LayoutListener = () => void;

function fakeApi(overrides: Record<string, unknown> = {}) {
  const listeners: { layoutFromJson?: LayoutListener; layoutChange?: LayoutListener } = {};
  const api = {
    onDidMovePanel: () => ({ dispose: () => {} }),
    onDidRemovePanel: () => ({ dispose: () => {} }),
    onDidActiveGroupChange: () => ({ dispose: () => {} }),
    activeGroup: null,
    onDidOpenPopoutWindowFail: () => ({ dispose: () => {} }),
    onDidLayoutFromJSON: (cb: LayoutListener) => {
      listeners.layoutFromJson = cb;
      return { dispose: () => delete listeners.layoutFromJson };
    },
    onDidLayoutChange: (cb: LayoutListener) => {
      listeners.layoutChange = cb;
      return { dispose: () => delete listeners.layoutChange };
    },
    fromJSON: vi.fn(),
    toJSON: () => ({ grid: { root: "settled" } }),
    getPanel: () => undefined,
    ...overrides,
  };
  return {
    api: api as unknown as DockviewApi,
    fire: (event: keyof typeof listeners) => listeners[event]?.(),
    raw: api,
  };
}

function savedLayout(schemaVersion: number): string {
  return JSON.stringify({ schemaVersion, layout: { grid: { root: "saved" } } });
}

describe("startWorkbench — restore gates", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      return setTimeout(() => cb(0), 0) as unknown as number;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  });

  afterEach(() => {
    stopWorkbench();
    vi.unstubAllGlobals();
    vi.useRealTimers();
    localStorage.clear();
  });

  it("rebuilds instead of loading a snapshot from an older schema", () => {
    localStorage.setItem(STORAGE_KEY, savedLayout(LAYOUT_SCHEMA_VERSION - 1));
    const { api, raw } = fakeApi();

    startWorkbench(api);

    expect((raw.fromJSON as ReturnType<typeof vi.fn>)).not.toHaveBeenCalled();
    expect(recoverDefaultLayout).toHaveBeenCalledWith(api);
  });

  it("falls back to the default layout when the saved snapshot cannot be restored", () => {
    localStorage.setItem(STORAGE_KEY, savedLayout(LAYOUT_SCHEMA_VERSION));
    const { api, raw, fire } = fakeApi({
      fromJSON: vi.fn(() => {
        throw new Error("bad snapshot");
      }),
    });

    startWorkbench(api);
    fire("layoutFromJson");

    expect((raw.fromJSON as ReturnType<typeof vi.fn>)).toHaveBeenCalledTimes(1);
    expect(recoverDefaultLayout).toHaveBeenCalled();
  });

  it("does not persist layout changes while a restore is pending, then saves once it settles", () => {
    vi.useFakeTimers();
    localStorage.setItem(STORAGE_KEY, savedLayout(LAYOUT_SCHEMA_VERSION));
    const { api, fire } = fakeApi();

    startWorkbench(api);

    fire("layoutChange");
    vi.advanceTimersByTime(600);
    expect(localStorage.getItem(STORAGE_KEY)).toBe(savedLayout(LAYOUT_SCHEMA_VERSION));

    fire("layoutFromJson");
    fire("layoutChange");
    vi.advanceTimersByTime(600);

    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}");
    expect(saved.schemaVersion).toBe(LAYOUT_SCHEMA_VERSION);
    expect(saved.layout).toEqual({ grid: { root: "settled" } });
  });
});
