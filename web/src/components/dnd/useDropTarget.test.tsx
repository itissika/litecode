import { act, renderHook } from "@testing-library/react";
import type { DragEvent } from "react";
import { describe, expect, it, vi } from "vitest";

import { useDropTarget } from "./useDropTarget";

const RECT = { top: 0, height: 40 } as DOMRect;

function dragEvent(
  types: string[],
  data: Record<string, string> = {},
  init: { clientY?: number; relatedTarget?: Node | null; contains?: boolean } = {},
): DragEvent<HTMLElement> {
  return {
    dataTransfer: {
      types,
      dropEffect: "none",
      getData: (mime: string) => data[mime] ?? "",
    },
    currentTarget: {
      getBoundingClientRect: () => RECT,
      contains: () => init.contains ?? false,
    },
    clientY: init.clientY ?? 0,
    relatedTarget: init.relatedTarget ?? null,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as DragEvent<HTMLElement>;
}

function setup(onDrop = vi.fn(), canDrop?: () => boolean) {
  return renderHook(() =>
    useDropTarget<{ id: string }>({
      mime: "x/test",
      parse: (raw) => (raw ? { id: raw } : null),
      resolve: (event, rect) =>
        event.clientY < rect.top + rect.height / 2 ? "before" : "after",
      canDrop,
      onDrop,
    }),
  );
}

describe("useDropTarget", () => {
  it("resolves the position and drops the parsed payload", () => {
    const onDrop = vi.fn();
    const { result } = setup(onDrop);

    act(() =>
      result.current.handlers.onDragOver(
        dragEvent(["x/test"], { "x/test": "s1" }, { clientY: 5 }),
      ),
    );
    expect(result.current.isOver).toBe(true);
    expect(result.current.position).toBe("before");

    act(() =>
      result.current.handlers.onDrop(
        dragEvent(["x/test"], { "x/test": "s1" }, { clientY: 5 }),
      ),
    );
    expect(onDrop).toHaveBeenCalledWith(
      { id: "s1" },
      "before",
      expect.objectContaining({ clientY: 5 }),
    );
    expect(result.current.position).toBeNull();
  });

  it("ignores a foreign MIME", () => {
    const { result } = setup();
    const event = dragEvent(["text/plain"]);
    act(() => result.current.handlers.onDragOver(event));
    expect(result.current.isOver).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it("keeps the hover when leaving to a child, clears when leaving the row", () => {
    const { result } = setup();
    act(() =>
      result.current.handlers.onDragEnter(
        dragEvent(["x/test"], { "x/test": "s1" }),
      ),
    );
    expect(result.current.isOver).toBe(true);

    const child = document.createElement("span");
    act(() =>
      result.current.handlers.onDragLeave(
        dragEvent([], {}, { contains: true, relatedTarget: child }),
      ),
    );
    expect(result.current.isOver).toBe(true);

    act(() => result.current.handlers.onDragLeave(dragEvent([], {})));
    expect(result.current.isOver).toBe(false);
  });

  it("refuses the drop when canDrop says no", () => {
    const onDrop = vi.fn();
    const { result } = setup(onDrop, () => false);
    act(() =>
      result.current.handlers.onDrop(
        dragEvent(["x/test"], { "x/test": "s1" }),
      ),
    );
    expect(onDrop).not.toHaveBeenCalled();
  });
});
