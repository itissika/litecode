import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  DEBUG_STORAGE_KEY,
  debugEnabled,
  parseDebugSpec,
  setDebugSpec,
} from "./debugTrace";

const store = new Map<string, string>();

beforeEach(() => {
  store.clear();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
    removeItem: (k: string) => {
      store.delete(k);
    },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("parseDebugSpec", () => {
  it("is off for empty / explicit off", () => {
    expect([...parseDebugSpec(null)]).toEqual([]);
    expect([...parseDebugSpec("")]).toEqual([]);
    expect([...parseDebugSpec("off")]).toEqual([]);
    expect([...parseDebugSpec("0")]).toEqual([]);
  });

  it("enables both channels for * / on / 1", () => {
    expect([...parseDebugSpec("*")].sort()).toEqual(["buffer", "turn"]);
    expect([...parseDebugSpec("on")].sort()).toEqual(["buffer", "turn"]);
    expect([...parseDebugSpec("1")].sort()).toEqual(["buffer", "turn"]);
  });

  it("selects listed channels", () => {
    expect([...parseDebugSpec("turn")]).toEqual(["turn"]);
    expect([...parseDebugSpec("turn,buffer")].sort()).toEqual(["buffer", "turn"]);
  });
});

describe("debugEnabled", () => {
  it("defaults off", () => {
    expect(debugEnabled("turn")).toBe(false);
    expect(debugEnabled("buffer")).toBe(false);
  });

  it("reads localStorage spec", () => {
    setDebugSpec("turn");
    expect(store.get(DEBUG_STORAGE_KEY)).toBe("turn");
    expect(debugEnabled("turn")).toBe(true);
    expect(debugEnabled("buffer")).toBe(false);
  });
});
