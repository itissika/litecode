import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  EnginesDetail,
  RetrievalCorpusStatus,
} from "../../../../api/workspace";
import * as workspace from "../../../../api/workspace";
import { useSettingsStore } from "../../../../stores/settingsStore";
import { EngineView } from "./EngineView";
import {
  EnginesSection,
  FAST_CYCLES_AFTER_REFRESH,
  POLL_FAST_MS,
  POLL_IDLE_MS,
} from "./EnginesSection";
import { retrievalBar, retrievalTip } from "./retrievalBar";

vi.mock("./EmbedModelDownload", () => ({
  useEmbedModelDownload: () => ({ available: false, state: { phase: "idle" }, start: vi.fn() }),
  EmbedModelDownloadCard: () => null,
}));

function c(p: Partial<RetrievalCorpusStatus> = {}): RetrievalCorpusStatus {
  return {
    state: "ready",
    phase: null,
    chunks_done: 100,
    chunks_total: 100,
    chunks_pending: 0,
    files_pending: 0,
    error: null,
    ...p,
  };
}

function detail(
  total: RetrievalCorpusStatus,
  code: RetrievalCorpusStatus = total,
  session: RetrievalCorpusStatus = c(),
  usable: EnginesDetail["retrieval"]["usable"] = "ready",
): EnginesDetail {
  return {
    retrieval: {
      desired: true,
      state: "warm" as never,
      usable,
      error: null,
      model: { model_found: true, tokenizer_found: true, ready: true, model_dir: "/m/x" },
      total,
      code,
      session,
      index: {
        exists: true,
        needs_rebuild: false,
        vectors_ready: true,
        indexed_files: 1,
      },
      policy: { product_internal_dirs: [], exclude_globs: [], max_file_bytes: 0, binary_files: false },
    },
    lsp: {
      desired: false,
      usable: "stopped",
      configured_servers: [],
      error: null,
      probes: [],
    },
  };
}

const fill = () => screen.getByTestId("engine-index-fill");
const track = () => screen.getByTestId("engine-index-track");

beforeEach(() => {
  useSettingsStore.setState({
    persistByDoc: {},
    engines: { version: 1, lsp: { desired: false, servers: [] }, retrieval: { desired: true } },
    saveEngines: vi.fn(async () => undefined),
  } as never);
  vi.spyOn(workspace, "probeLspServers").mockResolvedValue([]);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("WP-136 retrieval bar — tones", () => {
  it("1a failed → red, not pulsing", () => {
    const err = { engine: "code" as const, reason: "index_failed" as const, message: "boom" };
    render(<EngineView detail={detail(c({ state: "failed", error: err }), c({ state: "failed", error: err }))} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("failed");
    expect(fill().className).not.toContain("is-pulse");
  });
  it("1b needs_rebuild → empty bar (0%)", () => {
    render(<EngineView detail={detail(c({ state: "needs_rebuild", chunks_done: 0 }))} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("empty");
    expect(fill().style.width).toBe("0%");
  });
  it("1c refreshing+embedding → orange pulse with done/total", () => {
    render(<EngineView detail={detail(c({ state: "refreshing", phase: "embedding", chunks_done: 30, chunks_total: 120 }))} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("busy");
    expect(fill().className).toContain("is-pulse");
    expect(fill().style.width).toBe("25%");
  });
  it("1d ready + pending=0 → green 100%", () => {
    render(<EngineView detail={detail(c())} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("ok");
    expect(fill().style.width).toBe("100%");
  });
  it("2 ready + files_pending>0 → orange, NOT pulsing", () => {
    render(<EngineView detail={detail(c({ files_pending: 4, chunks_done: 96 }))} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("pending");
    expect(fill().className).not.toContain("is-pulse");
  });
});

describe("WP-142 files_pending drives the pending orange", () => {
  it("files_pending>0 && chunks_pending=0 → orange, no pulse", () => {
    const b = retrievalBar({ total: c({ files_pending: 3, chunks_pending: 0 }), code: c(), session: c() });
    expect(b.tone).toBe("pending");
    expect(b.pulse).toBe(false);
  });
  it("files_pending=0 && chunks_pending>0 → not orange (green, real pct)", () => {
    const t = c({ files_pending: 0, chunks_pending: 20, chunks_done: 80, chunks_total: 100 });
    const b = retrievalBar({ total: t, code: c(), session: c() });
    expect(b.tone).toBe("ok");
    expect(b.pct).toBe(80); // Q3: no fake fill
  });
  it("refreshing + files_pending>0 → still busy pulse (Q1)", () => {
    const b = retrievalBar({
      total: c({ state: "refreshing", phase: "embedding", chunks_done: 5, chunks_total: 10, files_pending: 7 }),
      code: c(),
      session: c(),
    });
    expect(b.tone).toBe("busy");
    expect(b.pulse).toBe(true);
  });
  it("files_pending never changes the percentage or hover", () => {
    const a = c({ chunks_done: 50, chunks_total: 100 });
    const r1 = { total: a, code: a, session: c() };
    const r2 = { total: { ...a, files_pending: 9 }, code: { ...a, files_pending: 9 }, session: c() };
    expect(retrievalBar(r2).pct).toBe(retrievalBar(r1).pct);
    expect(retrievalTip(r2)).toBe(retrievalTip(r1));
  });
});

describe("WP-136 scanning / incremental / red priority / hover", () => {
  it("3 scanning (chunks_total=null) → indeterminate, no %, no 0%", () => {
    const scan = c({ state: "refreshing", phase: "scanning", chunks_total: null, chunks_done: 0 });
    render(<EngineView detail={detail(scan, scan)} onChanged={() => {}} />);
    expect(fill().dataset.indeterminate).toBe("true");
    expect(fill().className).toContain("is-indeterminate");
    expect(fill().style.width).toBe("");
    const tip = track().getAttribute("title") ?? "";
    expect(tip.split("\n")[0]).toBe("Total scanning");
    expect(tip).not.toMatch(/Total 0%/);
  });
  it("4 incremental: whole-index denominator → ~98%", () => {
    const t = c({ state: "refreshing", phase: "embedding", chunks_done: 980, chunks_total: 1000 });
    expect(retrievalBar({ total: t, code: t, session: c() }).pct).toBe(98);
    render(<EngineView detail={detail(t)} onChanged={() => {}} />);
    expect(fill().style.width).toBe("98%");
  });
  it("5 red priority + hover names engine and reason", () => {
    const reasons = [
      ["model_missing", "模型缺失"],
      ["engine_unavailable", "引擎不可用"],
      ["engine_stopped", "引擎已停止"],
    ] as const;
    for (const [reason, text] of reasons) {
      const err = { engine: "session" as const, reason, message: "m" };
      const failedSide = c({ state: "failed", error: err });
      const r = { total: c({ state: "failed", error: err }), code: c(), session: failedSide };
      expect(retrievalBar(r).tone).toBe("failed");
      const tip = retrievalTip(r);
      expect(tip).toContain("session");
      expect(tip).toContain(text);
    }
    // code failed, session ready → red even if total said otherwise
    const err = { engine: "code" as const, reason: "engine_stopped" as const, message: "" };
    expect(retrievalBar({ total: c(), code: c({ state: "failed", error: err }), session: c() }).tone).toBe("failed");
  });
  it("6 hover three lines", () => {
    const r = {
      total: c({ chunks_done: 150, chunks_total: 200, chunks_pending: 50 }),
      code: c({ chunks_done: 100, chunks_total: 100 }),
      session: c({ chunks_done: 50, chunks_total: 100, chunks_pending: 50 }),
    };
    expect(retrievalTip(r).split("\n")).toEqual([
      "Total 75%",
      "Code 100% · 100/100 chunks",
      "Session 50% · 50/100 chunks",
    ]);
  });
});

describe("WP-136 rebuild click + polling", () => {
  it("7 click does not change colour; orange only once backend says refreshing; repeated needs_rebuild stays empty and clickable", async () => {
    const refresh = vi.spyOn(workspace, "refreshRetrieval").mockResolvedValue({ desired: true, mode: "rebuild" });
    const nr = detail(c({ state: "needs_rebuild", chunks_done: 0 }));
    const view = render(<EngineView detail={nr} onChanged={() => {}} />);
    const btn = screen.getByLabelText("Refresh index") as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(btn);
    });
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(fill().dataset.tone).toBe("empty");
    // backend still needs_rebuild (3 cycles) → still empty, still clickable
    for (let i = 0; i < 3; i++) view.rerender(<EngineView detail={detail(c({ state: "needs_rebuild", chunks_done: 0 }))} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("empty");
    expect((screen.getByLabelText("Refresh index") as HTMLButtonElement).disabled).toBe(false);
    // backend now refreshing → orange pulse
    view.rerender(<EngineView detail={detail(c({ state: "refreshing", phase: "embedding", chunks_done: 1, chunks_total: 10 }))} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("busy");
    expect(fill().className).toContain("is-pulse");
  });

  it("8 polling: idle 5s, refreshing 1s, after refresh click 1s for N cycles, hidden stops", async () => {
    vi.useFakeTimers();
    const getDetail = vi.spyOn(workspace, "getEnginesDetail").mockResolvedValue(detail(c()));
    render(<EnginesSection />);
    await act(async () => undefined);
    expect(getDetail).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_FAST_MS); });
    expect(getDetail).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_IDLE_MS - POLL_FAST_MS); });
    expect(getDetail).toHaveBeenCalledTimes(2);

    // refreshing → 1s
    getDetail.mockResolvedValue(detail(c({ state: "refreshing", phase: "embedding", chunks_done: 1, chunks_total: 2 })));
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_IDLE_MS); });
    const n = getDetail.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_FAST_MS); });
    expect(getDetail.mock.calls.length).toBe(n + 1);

    // idle again + refresh click → 1s for FAST_CYCLES_AFTER_REFRESH cycles
    getDetail.mockResolvedValue(detail(c({ state: "needs_rebuild", chunks_done: 0 })));
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_FAST_MS); });
    vi.spyOn(workspace, "refreshRetrieval").mockResolvedValue({ desired: true, mode: "rebuild" });
    await act(async () => { fireEvent.click(screen.getByLabelText("Refresh index")); });
    const m = getDetail.mock.calls.length;
    for (let i = 0; i < FAST_CYCLES_AFTER_REFRESH; i++) {
      await act(async () => { await vi.advanceTimersByTimeAsync(POLL_FAST_MS); });
    }
    expect(getDetail.mock.calls.length).toBe(m + FAST_CYCLES_AFTER_REFRESH);
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_FAST_MS); });
    expect(getDetail.mock.calls.length).toBe(m + FAST_CYCLES_AFTER_REFRESH);

    // hidden → stops
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); });
    const h = getDetail.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(POLL_IDLE_MS * 3); });
    expect(getDetail.mock.calls.length).toBe(h);
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  });
});

describe("WP-136 no legacy fields", () => {
  it("9 contradictory legacy values are ignored", () => {
    const d = detail(c({ state: "needs_rebuild", chunks_done: 0 }));
    // legacy shapes smuggled in at runtime must not influence the bar
    Object.assign(d.retrieval.index as object, {
      status: "ready",
      progress: { phase: "embedding", files_done: 45, files_total: 100, chunks_done: 9 },
      indexed_chunks: 999,
      sync_pct: 100,
      pending_updates: 999,
      session: { status: "ready", pending_updates: 999 },
    });
    render(<EngineView detail={d} onChanged={() => {}} />);
    expect(fill().dataset.tone).toBe("empty");
    expect(fill().style.width).toBe("0%");
    expect(track().getAttribute("title")).not.toMatch(/45|999/);
  });
});
