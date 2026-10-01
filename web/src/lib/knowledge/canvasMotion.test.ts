import { afterEach, describe, expect, it, vi } from "vitest";

import {
  CANVAS_IDLE_MS,
  createCanvasMotion,
  intersectsViewport,
  isCanvasGesture,
  zoomChanged,
} from "./canvasMotion";

describe("zoomChanged", () => {
  it("ignores pan and the first sample", () => {
    expect(zoomChanged(null, 1)).toBe(false);
    expect(zoomChanged(1, 1)).toBe(false);
  });

  it("notices a zoom change", () => {
    expect(zoomChanged(1, 1.2)).toBe(true);
  });
});

describe("isCanvasGesture", () => {
  it("includes drag and resize, including the release event", () => {
    expect(isCanvasGesture({ type: "position", dragging: true })).toBe(true);
    expect(isCanvasGesture({ type: "position", dragging: false })).toBe(true);
    expect(isCanvasGesture({ type: "dimensions", resizing: true })).toBe(true);
    expect(isCanvasGesture({ type: "dimensions", resizing: false })).toBe(true);
  });

  it("ignores selection and measurement", () => {
    expect(isCanvasGesture({ type: "position" })).toBe(false);
    expect(isCanvasGesture({ type: "dimensions" })).toBe(false);
    expect(isCanvasGesture({ type: "select", dragging: true })).toBe(false);
  });
});

describe("intersectsViewport", () => {
  const view = new DOMRect(0, 0, 800, 600);

  it("keeps a card that overlaps the pane", () => {
    expect(intersectsViewport(new DOMRect(760, 10, 80, 40), view)).toBe(true);
  });

  it("drops a card fully outside the pane", () => {
    expect(intersectsViewport(new DOMRect(900, 10, 80, 40), view)).toBe(false);
  });
});

describe("createCanvasMotion", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function hostWithCards() {
    const root = document.createElement("div");
    vi.spyOn(root, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 800, 600));
    const visible = document.createElement("div");
    visible.className = "knowledge-flow-card";
    const hidden = document.createElement("div");
    hidden.className = "knowledge-flow-card";
    const arriving = document.createElement("div");
    arriving.className = "knowledge-flow-card is-arrive";
    vi.spyOn(visible, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 120, 80));
    vi.spyOn(hidden, "getBoundingClientRect").mockReturnValue(new DOMRect(2000, 10, 120, 80));
    vi.spyOn(arriving, "getBoundingClientRect").mockReturnValue(new DOMRect(20, 20, 120, 80));
    const cancel = vi.fn();
    const animate = vi.fn(() => ({ cancel }) as unknown as Animation);
    for (const card of [visible, hidden, arriving]) {
      card.animate = animate;
      root.append(card);
    }
    return { root, visible, animate, cancel };
  }

  it("stays cheap until the idle delay, then sharpens on-screen cards", () => {
    vi.useFakeTimers();
    const motion = createCanvasMotion();
    const { root, animate } = hostWithCards();

    motion.note(root);
    vi.advanceTimersByTime(CANVAS_IDLE_MS - 1);
    expect(animate).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(animate).toHaveBeenCalledTimes(1);
    expect(animate).toHaveBeenCalledWith(
      expect.any(Array),
      expect.objectContaining({ fill: "none" }),
    );
  });

  it("restarts the idle wait while the gesture continues", () => {
    vi.useFakeTimers();
    const motion = createCanvasMotion();
    const { root, animate } = hostWithCards();

    motion.note(root);
    vi.advanceTimersByTime(CANVAS_IDLE_MS - 1);
    motion.note(root);
    vi.advanceTimersByTime(CANVAS_IDLE_MS - 1);
    expect(animate).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(animate).toHaveBeenCalledTimes(1);
  });

  it("cancels a sharpen if the gesture starts again", () => {
    vi.useFakeTimers();
    const motion = createCanvasMotion();
    const { root, animate, cancel } = hostWithCards();

    motion.note(root);
    vi.advanceTimersByTime(CANVAS_IDLE_MS);
    expect(animate).toHaveBeenCalledTimes(1);

    motion.note(root);
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("does not sharpen after dispose", () => {
    vi.useFakeTimers();
    const motion = createCanvasMotion();
    const { root, animate } = hostWithCards();

    motion.note(root);
    motion.dispose();
    vi.advanceTimersByTime(CANVAS_IDLE_MS);
    expect(animate).not.toHaveBeenCalled();
  });
});
