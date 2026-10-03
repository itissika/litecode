import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useSessionStore } from "../stores/sessionStore";
import { useSettingsStore } from "../stores/settingsStore";
import { ModelSwitcher } from "./ModelSwitcher";

const MODELS = [
  {
    id: "openai/alpha",
    api_model_id: "alpha-model",
    provider_id: "openai",
    label: "Alpha",
    context_window: 1000,
  },
  {
    id: "openai/beta",
    api_model_id: "beta-model",
    provider_id: "openai",
    label: "Beta",
    context_window: 1000,
  },
  {
    id: "deepseek/gamma",
    api_model_id: "gamma-model",
    provider_id: "deepseek",
    label: "Gamma",
    context_window: 1000,
  },
];

beforeEach(() => {
  useSessionStore.setState({
    availableModels: MODELS,
    byId: new Map([
      ["session-1", { modelId: "openai/alpha", label: "Alpha" }],
    ]),
  } as never);
  useSettingsStore.setState({
    llm: {
      providers: [
        { id: "openai", name: "OpenAI" },
        { id: "deepseek", name: "DeepSeek" },
      ],
    },
  } as never);
});

afterEach(() => {
  cleanup();
  useSessionStore.setState({ availableModels: [], byId: new Map() } as never);
  useSettingsStore.setState({ llm: null } as never);
});

function boxRect(
  top: number,
  bottom: number,
  left = 0,
  right = 100,
): DOMRect {
  return {
    top,
    bottom,
    left,
    right,
    width: right - left,
    height: bottom - top,
    x: left,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

describe("ModelSwitcher filters", () => {
  it("reorders and dims rows instead of dropping them", () => {
    render(<ModelSwitcher sessionId="session-1" />);
    fireEvent.click(screen.getByRole("button", { name: "Alpha" }));

    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    const menu = within(panel);
    const providerFilter = menu.getByRole("button", {
      name: "Filter models from OpenAI",
    });
    expect(providerFilter.getAttribute("aria-pressed")).toBe("false");
    expect(menu.getByTitle("OpenAI").getAttribute("data-provider-logo")).toBe(
      "brand",
    );

    const search = screen.getByPlaceholderText("Filter models");
    const rowButtons = () => Array.from(panel.querySelectorAll("button"));
    const names = () =>
      rowButtons()
        .map((button) => button.textContent?.trim() ?? "")
        .filter((text) => text.length > 0);
    const dimmed = (name: string) =>
      rowButtons()
        .find((button) => button.textContent?.trim() === name)
        ?.classList.contains("opacity-40") ?? false;

    expect(names()).toEqual(["Alpha", "Beta", "Gamma"]);

    // Provider filter keeps every row: the others sink and dim.
    fireEvent.click(providerFilter);
    expect(providerFilter.getAttribute("aria-pressed")).toBe("true");
    expect(names()).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(dimmed("Alpha")).toBe(false);
    expect(dimmed("Gamma")).toBe(true);

    // Text query lifts matches inside a group and dims the rest.
    fireEvent.change(search, { target: { value: "beta" } });
    expect(names()).toEqual(["Beta", "Alpha", "Gamma"]);
    expect(dimmed("Beta")).toBe(false);
    expect(dimmed("Alpha")).toBe(true);
    expect(dimmed("Gamma")).toBe(true);

    // Single-select: picking another provider switches the filter.
    fireEvent.change(search, { target: { value: "" } });
    const deepseekFilter = menu.getByRole("button", {
      name: "Filter models from DeepSeek",
    });
    fireEvent.click(deepseekFilter);
    expect(deepseekFilter.getAttribute("aria-pressed")).toBe("true");
    expect(providerFilter.getAttribute("aria-pressed")).toBe("false");
    expect(names()).toEqual(["Gamma", "Alpha", "Beta"]);
    expect(dimmed("Gamma")).toBe(false);
    expect(dimmed("Alpha")).toBe(true);

    // Clicking the selected provider clears the filter.
    fireEvent.click(deepseekFilter);
    expect(deepseekFilter.getAttribute("aria-pressed")).toBe("false");
    expect(names()).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(dimmed("Alpha")).toBe(false);
    expect(dimmed("Gamma")).toBe(false);
  });

  it("shows what each model takes beyond text, on the trigger and every row", () => {
    useSessionStore.setState({
      availableModels: [
        { ...MODELS[0], modalities: ["text", "image", "pdf"] },
        { ...MODELS[1], modalities: ["text"] },
      ],
    } as never);

    render(<ModelSwitcher sessionId="session-1" />);
    const glyphs = (root: HTMLElement | Document) =>
      Array.from(root.querySelectorAll('[role="img"]')).map((glyph) =>
        glyph.getAttribute("aria-label"),
      );

    // The trigger carries the session's own model — no need to open the list to
    // see whether the composer's images can be read.
    const trigger = screen.getByTitle("Model: Alpha");
    expect(glyphs(trigger)).toEqual(["image", "pdf"]);

    fireEvent.click(trigger);
    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    // Every row is marked in the catalog's own order, and a text-only model
    // stays bare instead of getting a guessed glyph.
    expect(glyphs(panel)).toEqual(["image", "pdf"]);
    expect(
      within(panel)
        .getByTitle("Accepts image input")
        .getAttribute("aria-label"),
    ).toBe("image");
  });

  it("clamps the list to the pane that hosts the composer, not the window", () => {
    render(
      <div className="dv-groupview">
        <div className="dv-content-container" data-testid="pane">
          <ModelSwitcher sessionId="session-1" />
        </div>
      </div>,
    );
    // A pane that is far shorter than the window. The trigger sits near its
    // bottom, so the menu opens upward into the pane.
    screen.getByTestId("pane").getBoundingClientRect = () =>
      boxRect(0, 300, 0, window.innerWidth);
    const trigger = screen.getByRole("button", { name: "Alpha" });
    const at = boxRect(270, 290);
    trigger.getBoundingClientRect = () => at;
    trigger.parentElement!.getBoundingClientRect = () => at;

    fireEvent.click(trigger);

    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    // 270px above the trigger inside the pane, less the margin.
    expect(panel.style.maxHeight).toBe("262px");
    expect(panel.style.bottom).not.toBe("");
    expect(window.innerHeight).toBeGreaterThan(262);
  });

  it("opens downward when that side of the pane is the taller one", () => {
    render(
      <div className="dv-groupview">
        <div className="dv-content-container" data-testid="pane">
          <ModelSwitcher sessionId="session-1" />
        </div>
      </div>,
    );
    const vh = window.innerHeight;
    expect(vh).toBeGreaterThan(700);
    screen.getByTestId("pane").getBoundingClientRect = () =>
      boxRect(0, vh, 0, window.innerWidth);
    const trigger = screen.getByRole("button", { name: "Alpha" });
    const at = boxRect(220, 240);
    trigger.getBoundingClientRect = () => at;
    trigger.parentElement!.getBoundingClientRect = () => at;

    fireEvent.click(trigger);

    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    expect(panel.style.top).not.toBe("");
    expect(panel.style.bottom).toBe("");
    expect(panel.style.maxHeight).toBe(`${vh - 240 - 8}px`);
  });

  it("ignores pane space that sits outside the viewport", () => {
    render(
      <div className="dv-groupview">
        <div className="dv-content-container" data-testid="pane">
          <ModelSwitcher sessionId="session-1" />
        </div>
      </div>,
    );
    // The pane starts above the screen. The visible gap above the trigger is
    // what limits an upward menu, not the off-screen part of the pane.
    screen.getByTestId("pane").getBoundingClientRect = () =>
      boxRect(-180, 500, 0, window.innerWidth);
    const trigger = screen.getByRole("button", { name: "Alpha" });
    const at = boxRect(320, 340);
    trigger.getBoundingClientRect = () => at;
    trigger.parentElement!.getBoundingClientRect = () => at;

    fireEvent.click(trigger);

    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    expect(panel.style.bottom).not.toBe("");
    expect(panel.style.maxHeight).toBe("312px");
  });

  it("opens into a short side without a taller floor", () => {
    render(
      <div className="dv-groupview">
        <div className="dv-content-container" data-testid="pane">
          <ModelSwitcher sessionId="session-1" />
        </div>
      </div>,
    );
    screen.getByTestId("pane").getBoundingClientRect = () =>
      boxRect(0, 80, 0, window.innerWidth);
    const trigger = screen.getByRole("button", { name: "Alpha" });
    const at = boxRect(20, 40);
    trigger.getBoundingClientRect = () => at;
    trigger.parentElement!.getBoundingClientRect = () => at;

    fireEvent.click(trigger);

    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    // Opens downward into 40px.
    expect(panel.style.maxHeight).toBe("32px");
    expect(panel.style.top).not.toBe("");
    expect(panel.style.bottom).toBe("");
  });

  it("keeps a floor when the list height is dragged", async () => {
    render(<ModelSwitcher sessionId="session-1" />);
    fireEvent.click(screen.getByRole("button", { name: "Alpha" }));

    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    const body = panel.firstElementChild as HTMLElement;
    expect(body.style.height).toBe("");
    const grip = within(panel).getByTitle("Drag to resize the list");

    fireEvent.pointerDown(grip, { pointerId: 1, clientY: 400 });
    fireEvent.pointerMove(grip, { pointerId: 1, clientY: 600 });
    await waitFor(() => expect(body.style.height).toBe("96px"));
    fireEvent.pointerUp(grip, { pointerId: 1, clientY: 600 });

    // The drag is done; the panel keeps the height it was left at.
    expect(body.style.height).toBe("96px");
  });
});

describe("ModelSwitcher field variant", () => {
  const models = [
    {
      id: "prov/m1",
      api_model_id: "gpt-4o",
      provider_id: "prov",
      label: "GPT",
      context_window: 1000,
    },
  ];

  it("uses the settings underline and the overlay menu, not the composer chip", () => {
    useSettingsStore.setState({
      llm: { providers: [{ id: "prov", name: "Prov" }] },
    } as never);
    render(
      <ModelSwitcher
        variant="field"
        models={models}
        modelId="prov/m1"
        onChange={() => {}}
      />,
    );

    const trigger = screen.getByRole("button", { name: "GPT" });
    expect(trigger.className).toContain("border-b");
    expect(trigger.className).not.toContain("h-7");

    fireEvent.click(trigger);
    const panel = document.querySelector<HTMLElement>("[data-dropdown-panel]");
    if (!panel) throw new Error("model dropdown did not open");
    expect(panel.className).toContain("bg-(--_dk-overlay)");
    expect(panel.className).not.toContain("backdrop-blur");
    expect(panel.style.top).not.toBe("");
    expect(within(panel).getByText("Prov")).toBeTruthy();
  });
});
