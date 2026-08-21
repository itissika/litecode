import { cleanup, fireEvent, render, screen, act } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FoldCard } from "./FoldCard";
import { clearFoldCardOpen, requestFoldCardOpen } from "./foldCardState";

/**
 * FoldCard open/collapse state machine.
 *
 * Invariant under test: a remounted card (virtual-list scroll-out/in) restores
 * the exact state it had at unmount — persisted on EVERY change, including
 * while streaming. `defaultOpen` / `streaming` only decide the state of cards
 * that have never been persisted.
 */
const SESSION = "s1";
const ID = `${SESSION}:bubble:tool:call_1`;

function renderCard(props: {
  streaming?: boolean;
  defaultOpen?: boolean;
} = {}) {
  const { streaming, defaultOpen, ...rest } = props;
  return render(
    <FoldCard id={ID} label="bash" streaming={streaming} defaultOpen={defaultOpen} {...rest}>
      body
    </FoldCard>,
  );
}

function header() {
  return screen.getByRole("button", { name: "bash" });
}

afterEach(() => {
  cleanup();
  clearFoldCardOpen(SESSION);
});

describe("FoldCard mount defaults", () => {
  it("mounts collapsed when nothing else applies", () => {
    renderCard();
    expect(header().getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("body")).toBeNull();
  });

  it("mounts open when streaming with no persisted state", () => {
    renderCard({ streaming: true });
    // Live cards start ready, so the open state is visible immediately.
    expect(header().getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("body")).toBeTruthy();
  });

  it("mounts open when defaultOpen with no persisted state", () => {
    renderCard({ defaultOpen: true });
    expect(header().getAttribute("aria-expanded")).toBe("true");
  });
});

describe("FoldCard remount persistence", () => {
  it("keeps a card the user expanded open after remount", () => {
    const { unmount } = renderCard({ defaultOpen: true });
    fireEvent.click(header());
    expect(header().getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(header());
    expect(header().getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("body")).toBeTruthy();

    unmount();
    renderCard();
    expect(header().getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("body")).toBeTruthy();
  });

  it("keeps a card the user collapsed collapsed after remount, even with defaultOpen", () => {
    const { unmount } = renderCard({ defaultOpen: true });
    expect(header().getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(header());
    expect(header().getAttribute("aria-expanded")).toBe("false");

    unmount();
    // The old precedence bug: defaultOpen would re-expand the card on remount,
    // blowing up the bubble height and jittering the virtual list.
    renderCard({ defaultOpen: true });
    expect(header().getAttribute("aria-expanded")).toBe("false");
  });

  it("keeps a live card the user collapsed collapsed after remount while still streaming", () => {
    const { unmount } = renderCard({ streaming: true });
    expect(header().getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(header());
    expect(header().getAttribute("aria-expanded")).toBe("false");

    unmount();
    renderCard({ streaming: true });
    expect(header().getAttribute("aria-expanded")).toBe("false");
  });

  it("lets the system close an untouched card that ended while unmounted", () => {
    const { unmount } = renderCard({ streaming: true });
    expect(header().getAttribute("aria-expanded")).toBe("true");
    unmount();

    // No explicit user intent was stored, so the current system state owns the
    // remount rather than preserving a stale live measurement.
    renderCard({ streaming: false });
    expect(header().getAttribute("aria-expanded")).toBe("false");
  });
});

describe("FoldCard inner stick-to-bottom", () => {
  function scrollEl(): HTMLElement {
    return document.querySelector(".foldcard-scroll") as HTMLElement;
  }

  function mockOverflow(el: HTMLElement, scrollHeight = 400, clientHeight = 100) {
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => scrollHeight });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
  }

  it("starts pinned while streaming and pins to the bottom on growth", () => {
    const { rerender } = render(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1</div>
      </FoldCard>,
    );
    const el = scrollEl();
    mockOverflow(el);
    el.scrollTop = 300;

    rerender(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1 line2</div>
      </FoldCard>,
    );
    expect(el.scrollTop).toBe(400);
  });

  it("does not pin inner scroll after the user wheels up while streaming", () => {
    const { rerender } = render(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1</div>
      </FoldCard>,
    );
    const el = scrollEl();
    mockOverflow(el);
    el.scrollTop = 300;
    // Scroll-up gesture unpins synchronously — a flush in the same frame must
    // NOT yank the reader back to the bottom.
    fireEvent.wheel(el, { deltaY: -40 });

    rerender(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1 line2</div>
      </FoldCard>,
    );
    expect(el.scrollTop).toBe(300);
  });

  it("re-pins when the user wheels back to the bottom", () => {
    const raf = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((cb: FrameRequestCallback) => {
        cb(0);
        return 1;
      });
    const { rerender } = render(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1</div>
      </FoldCard>,
    );
    const el = scrollEl();
    mockOverflow(el);
    el.scrollTop = 300;
    fireEvent.wheel(el, { deltaY: -40 }); // unpin

    el.scrollTop = 400;
    fireEvent.wheel(el, { deltaY: 40 }); // scroll back to the end → re-pin
    raf.mockRestore();

    rerender(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1 line2</div>
      </FoldCard>,
    );
    expect(el.scrollTop).toBe(400);
  });

  it("does not re-pin on a streaming flip after the user unpinned", () => {
    const { rerender } = render(
      <FoldCard id={ID} label="bash" open streaming>
        <div>line1</div>
      </FoldCard>,
    );
    const el = scrollEl();
    mockOverflow(el);
    el.scrollTop = 300;
    fireEvent.wheel(el, { deltaY: -40 }); // unpin

    // A new live window (streaming false→true) must NOT yank the reader back.
    rerender(
      <FoldCard id={ID} label="bash" open streaming={false}>
        <div>line1 line2</div>
      </FoldCard>,
    );
    rerender(
      <FoldCard id={ID} label="bash" open streaming>
        <div>line1 line2 line3</div>
      </FoldCard>,
    );
    expect(el.scrollTop).toBe(300);
  });
});

describe("FoldCard streaming transitions", () => {
  it("follows the system state while the user has not chosen", () => {
    const { rerender } = renderCard({ streaming: false });
    expect(header().getAttribute("aria-expanded")).toBe("false");

    rerender(
      <FoldCard id={ID} label="bash" streaming>
        body
      </FoldCard>,
    );
    expect(header().getAttribute("aria-expanded")).toBe("true");

    rerender(
      <FoldCard id={ID} label="bash" streaming={false}>
        body
      </FoldCard>,
    );
    expect(header().getAttribute("aria-expanded")).toBe("false");
  });

  it("keeps a user-opened card open after the system work ends", () => {
    const { rerender } = renderCard({ streaming: true });
    fireEvent.click(header()); // explicit keepclosed
    fireEvent.click(header()); // explicit keepopen
    expect(header().getAttribute("aria-expanded")).toBe("true");

    rerender(
      <FoldCard id={ID} label="bash" streaming={false}>
        body
      </FoldCard>,
    );
    expect(header().getAttribute("aria-expanded")).toBe("true");
  });

  it("opens a mounted collapsed card when requested", () => {
    renderCard({ defaultOpen: true });
    fireEvent.click(header());
    expect(header().getAttribute("aria-expanded")).toBe("false");
    act(() => {
      requestFoldCardOpen(ID);
    });
    expect(header().getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("body")).toBeTruthy();
  });
});
