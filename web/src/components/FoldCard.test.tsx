import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { FoldCard } from "./FoldCard";
import { clearFoldCardOpen } from "./foldCardState";

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

  it("keeps a card open when its turn ended while it was unmounted (remount == unmount)", () => {
    const { unmount } = renderCard({ streaming: true });
    expect(header().getAttribute("aria-expanded")).toBe("true");
    unmount();

    // Turn ended while scrolled out of view: the auto-collapse effect never
    // ran. Remount must restore the open state so the measured height stays
    // valid — mounting collapsed would re-measure short and jitter the list.
    renderCard({ streaming: false });
    expect(header().getAttribute("aria-expanded")).toBe("true");
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

  it("does not pin inner scroll after the user leaves the bottom while streaming", () => {
    const { rerender } = render(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1</div>
      </FoldCard>,
    );
    const el = scrollEl();
    mockOverflow(el);
    el.scrollTop = 0;
    fireEvent.scroll(el);

    rerender(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1 line2</div>
      </FoldCard>,
    );
    expect(el.scrollTop).toBe(0);
  });

  it("keeps pinning while the user stays at the bottom", () => {
    const { rerender } = render(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1</div>
      </FoldCard>,
    );
    const el = scrollEl();
    mockOverflow(el);
    el.scrollTop = 300;
    fireEvent.scroll(el);

    rerender(
      <FoldCard id={ID} label="bash" streaming>
        <div>line1 line2</div>
      </FoldCard>,
    );
    expect(el.scrollTop).toBe(400);
  });
});

describe("FoldCard streaming transitions", () => {
  it("auto-collapses when streaming ends while mounted, and persists the collapse", () => {
    const { unmount, rerender } = renderCard({ streaming: true });
    expect(header().getAttribute("aria-expanded")).toBe("true");

    rerender(
      <FoldCard id={ID} label="bash" streaming={false}>
        body
      </FoldCard>,
    );
    expect(header().getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("body")).toBeNull();

    unmount();
    renderCard({ streaming: false });
    expect(header().getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("body")).toBeNull();
  });
});
