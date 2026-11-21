import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { PanelVisibilityProvider, usePanelVisible } from "./panelVisibility";

function Probe() {
  const visible = usePanelVisible();
  return <span data-testid="flag">{visible ? "visible" : "hidden"}</span>;
}

afterEach(cleanup);

describe("panel visibility", () => {
  it("defaults to visible when nothing provides a flag", () => {
    render(<Probe />);
    expect(screen.getByTestId("flag").textContent).toBe("visible");
  });

  it("reports an inactive panel", () => {
    render(
      <PanelVisibilityProvider active={false}>
        <Probe />
      </PanelVisibilityProvider>,
    );
    expect(screen.getByTestId("flag").textContent).toBe("hidden");
  });

  it("updates consumers when the panel activates", () => {
    const view = render(
      <PanelVisibilityProvider active={false}>
        <Probe />
      </PanelVisibilityProvider>,
    );
    view.rerender(
      <PanelVisibilityProvider active>
        <Probe />
      </PanelVisibilityProvider>,
    );
    expect(screen.getByTestId("flag").textContent).toBe("visible");
  });
});
