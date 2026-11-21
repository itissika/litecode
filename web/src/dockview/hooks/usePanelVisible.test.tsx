import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { usePanelVisible } from "./usePanelVisible";

type Handler = (event: { isVisible: boolean }) => void;

function fakeApi(initial: boolean) {
  const handlers = new Set<Handler>();
  const api = {
    isVisible: initial,
    onDidVisibilityChange(handler: Handler) {
      handlers.add(handler);
      return { dispose: () => handlers.delete(handler) };
    },
  };
  return {
    api,
    fire(isVisible: boolean) {
      api.isVisible = isVisible;
      for (const handler of handlers) handler({ isVisible });
    },
    listenerCount: () => handlers.size,
  };
}

function Harness({ api }: { api: { isVisible: boolean; onDidVisibilityChange: (h: Handler) => { dispose: () => void } } }) {
  const visible = usePanelVisible(api);
  return <span data-testid="visible">{visible ? "yes" : "no"}</span>;
}

afterEach(cleanup);

describe("usePanelVisible", () => {
  it("reads the panel's current visibility", () => {
    render(<Harness api={fakeApi(true).api} />);
    expect(screen.getByTestId("visible").textContent).toBe("yes");
  });

  it("follows the panel's visibility events", () => {
    const fake = fakeApi(true);
    render(<Harness api={fake.api} />);

    act(() => fake.fire(false));
    expect(screen.getByTestId("visible").textContent).toBe("no");

    act(() => fake.fire(true));
    expect(screen.getByTestId("visible").textContent).toBe("yes");
  });

  it("releases its subscription on unmount", () => {
    const fake = fakeApi(true);
    const view = render(<Harness api={fake.api} />);
    expect(fake.listenerCount()).toBe(1);

    view.unmount();
    expect(fake.listenerCount()).toBe(0);
  });
});
