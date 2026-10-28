import { afterEach, describe, expect, it } from "vitest";

import { observeHostSize, subscribeHostDocument } from "./hostDocument";

const cleanups: Array<() => void> = [];

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
});

/** A second document with a real window, the way a popout is one. */
function frame(): { doc: Document; view: Window } {
  const iframe = document.createElement("iframe");
  document.body.appendChild(iframe);
  const doc = iframe.contentDocument;
  const view = iframe.contentWindow;
  if (!doc || !view) throw new Error("popout document missing");
  cleanups.push(() => iframe.remove());
  return { doc, view };
}

async function settle(view: Window): Promise<void> {
  await Promise.resolve();
  await new Promise<void>((resolve) => {
    view.requestAnimationFrame(() => resolve());
  });
  await Promise.resolve();
}

function hostNode(): HTMLDivElement {
  const node = document.createElement("div");
  document.body.appendChild(node);
  cleanups.push(() => node.remove());
  return node;
}

describe("subscribeHostDocument", () => {
  it("reports the current window, then every move", async () => {
    const node = hostNode();
    const pop = frame();
    const seen: Window[] = [];
    cleanups.push(
      subscribeHostDocument(node, (win) => {
        seen.push(win);
      }),
    );

    expect(seen).toEqual([window]);

    pop.doc.body.appendChild(node);
    await settle(pop.view);
    expect(seen.at(-1)).toBe(pop.view);

    document.body.appendChild(node);
    await settle(window);
    expect(seen.at(-1)).toBe(window);
  });

  it("stops reporting once released", async () => {
    const node = hostNode();
    const pop = frame();
    const seen: Window[] = [];
    const stop = subscribeHostDocument(node, (win) => {
      seen.push(win);
    });
    stop();
    const before = seen.length;

    pop.doc.body.appendChild(node);
    await settle(pop.view);
    expect(seen.length).toBe(before);
  });
});

describe("observeHostSize", () => {
  it("fires now, rebuilds the observer on the new document, disconnects the old one", async () => {
    const node = hostNode();
    const pop = frame();
    let mainBuilt = 0;
    let mainDisconnected = 0;
    let popBuilt = 0;

    class MainObserver {
      constructor(readonly cb: ResizeObserverCallback) {}
      observe() {}
      unobserve() {}
      disconnect() {
        mainDisconnected += 1;
      }
    }
    class PopObserver {
      constructor(readonly cb: ResizeObserverCallback) {}
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    const scope = window as Window & { ResizeObserver?: typeof ResizeObserver };
    const previous = scope.ResizeObserver;
    scope.ResizeObserver = class extends MainObserver {
      constructor(cb: ResizeObserverCallback) {
        super(cb);
        mainBuilt += 1;
      }
    } as unknown as typeof ResizeObserver;
    (pop.view as Window & { ResizeObserver?: typeof ResizeObserver }).ResizeObserver =
      class extends PopObserver {
        constructor(cb: ResizeObserverCallback) {
          super(cb);
          popBuilt += 1;
        }
      } as unknown as typeof ResizeObserver;
    cleanups.push(() => {
      scope.ResizeObserver = previous;
    });

    let calls = 0;
    cleanups.push(observeHostSize(node, () => {
      calls += 1;
    }));

    expect(mainBuilt).toBe(1);
    expect(calls).toBe(1);

    pop.doc.body.appendChild(node);
    await settle(pop.view);

    expect(popBuilt).toBe(1);
    expect(mainDisconnected).toBe(1);
    expect(calls).toBe(2);
  });

  it("is a no-op for a detached node without a stream", () => {
    expect(() => observeHostSize(null, () => {})).not.toThrow();
  });
});
