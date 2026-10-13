import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AskQuestion } from "../api/types";
import { AskUserCard, OTHER_LABEL } from "./AskUserCard";

function renderCard(questions: AskQuestion[]) {
  const onGrant = vi.fn();
  render(
    <AskUserCard
      tool="ask_user"
      summary=""
      options={[]}
      multiSelect={false}
      questions={questions}
      onGrant={onGrant}
    />,
  );
  const submit = () =>
    screen.getByRole("button", { name: "Submit" }) as HTMLButtonElement;
  return { onGrant, submit };
}

const single: AskQuestion = {
  id: "s",
  prompt: "Pick one?",
  options: [
    { id: "a", label: "A" },
    { id: "b", label: "B" },
  ],
};
const multi: AskQuestion = { ...single, id: "m", multi_select: true };

describe("AskUserCard last-item free text (WP-082)", () => {
  afterEach(cleanup);

  it("1. every question ends in the input, whatever free_text says", async () => {
    const user = userEvent.setup();
    renderCard([
      { ...single, id: "q1", free_text: false },
      { ...single, id: "q2" },
      { ...single, id: "q3", free_text: true },
    ]);
    for (const id of ["q1", "q2", "q3"]) {
      const input = screen.getByTestId(`ask-user-free-text-${id}`);
      const item = screen.getByTestId(`ask-user-other-${id}`);
      expect(item.contains(input)).toBe(true);
      expect(item.parentElement?.lastElementChild).toBe(item);
      expect(input.hasAttribute("data-option-id")).toBe(false);
      await user.click(screen.getByRole("button", { name: "Next question" })).catch(() => {});
    }
  });

  it("2. label is 其他（自己填写）", () => {
    renderCard([single]);
    expect(OTHER_LABEL).toBe("其他（自己填写）");
    const input = screen.getByRole("textbox", { name: "其他（自己填写）" });
    expect(input.getAttribute("placeholder")).toBe("其他（自己填写）");
  });

  it("3a. single: typing after a pick clears it; submit sends selected []", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([single]);
    await user.click(screen.getByRole("button", { name: "A" }));
    await user.type(screen.getByTestId("ask-user-free-text-s"), "  mine ");
    expect(
      screen.getByRole("button", { name: "A" }).getAttribute("aria-pressed"),
    ).toBe("false");
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { s: { selected: [], free_text: "mine" } },
    });
  });

  it("3b. single: a pick after typing drops the text from the answer", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([single]);
    await user.type(screen.getByTestId("ask-user-free-text-s"), "mine");
    await user.click(screen.getByRole("button", { name: "B" }));
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { s: { selected: ["b"] } },
      selected: ["b"],
    });
  });

  it("4. multi: options and text coexist", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([multi]);
    await user.click(screen.getByRole("button", { name: "A" }));
    await user.type(screen.getByTestId("ask-user-free-text-m"), "also this");
    await user.click(screen.getByRole("button", { name: "B" }));
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { m: { selected: ["a", "b"], free_text: "also this" } },
      selected: ["a", "b"],
    });
  });

  it("5. text alone answers single and multi", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([single, multi]);
    expect(submit().disabled).toBe(true);
    await user.type(screen.getByTestId("ask-user-free-text-s"), "x");
    expect(submit().disabled).toBe(true);
    await user.click(screen.getByRole("button", { name: "Next question" }));
    await user.type(screen.getByTestId("ask-user-free-text-m"), "y");
    expect(submit().disabled).toBe(false);
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: {
        s: { selected: [], free_text: "x" },
        m: { selected: [], free_text: "y" },
      },
    });
  });

  it("6. whitespace only is unanswered and sends no free_text", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([multi]);
    await user.type(screen.getByTestId("ask-user-free-text-m"), "   ");
    expect(submit().disabled).toBe(true);
    await user.click(screen.getByRole("button", { name: "A" }));
    expect(submit().disabled).toBe(false);
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { m: { selected: ["a"] } },
      selected: ["a"],
    });
  });
});

describe("AskUserCard last item: visible label + selected state (WP-083)", () => {
  afterEach(cleanup);

  const other = (qid: string) => screen.getByTestId(`ask-user-other-${qid}`);
  const box = () => screen.getByRole("textbox", { name: OTHER_LABEL });
  const pressed = (name: string) =>
    screen.getByRole("button", { name }).getAttribute("aria-pressed");
  const looksOn = (el: HTMLElement) => {
    expect(el.classList.contains("btn-primary")).toBe(true);
    expect(el.classList.contains("btn-ghost")).toBe(false);
  };
  const looksOff = (el: HTMLElement) => {
    expect(el.classList.contains("btn-ghost")).toBe(true);
    expect(el.classList.contains("btn-primary")).toBe(false);
  };

  it("a. label text stays visible before and after typing", async () => {
    const user = userEvent.setup();
    renderCard([single]);
    expect(screen.getByText(OTHER_LABEL).tagName).toBe("SPAN");
    expect(box()).toBeTruthy();
    await user.type(box(), "mine");
    expect(screen.getByText(OTHER_LABEL)).toBeTruthy();
    expect(box()).toBeTruthy();
  });

  it("b. single: typing selects the last item like an option", async () => {
    const user = userEvent.setup();
    renderCard([single]);
    looksOff(other("s"));
    looksOff(screen.getByRole("button", { name: "A" }));
    await user.type(box(), "mine");
    expect(other("s").dataset.selected).toBe("true");
    const desc = document.getElementById(box().getAttribute("aria-describedby")!);
    expect(desc?.textContent).toBe("已选中");
    looksOn(other("s"));
    expect(pressed("A")).toBe("false");
    expect(pressed("B")).toBe("false");
  });

  it("c. single: a pick after typing deselects the last item", async () => {
    const user = userEvent.setup();
    renderCard([single]);
    await user.type(box(), "mine");
    await user.click(screen.getByRole("button", { name: "A" }));
    expect(other("s").dataset.selected).toBe("false");
    expect(box().getAttribute("aria-describedby")).toBeNull();
    looksOff(other("s"));
    expect(pressed("A")).toBe("true");
    looksOn(screen.getByRole("button", { name: "A" }));
  });

  it("d. single: whitespace only leaves the last item unselected", async () => {
    const user = userEvent.setup();
    renderCard([single]);
    await user.type(box(), "   ");
    expect(other("s").dataset.selected).toBe("false");
    expect(box().getAttribute("aria-describedby")).toBeNull();
    looksOff(other("s"));
  });

  it("e. multi: selected options and the last item coexist", async () => {
    const user = userEvent.setup();
    renderCard([multi]);
    looksOff(other("m"));
    await user.click(screen.getByRole("button", { name: "A" }));
    await user.type(box(), "also");
    expect(pressed("A")).toBe("true");
    expect(other("m").dataset.selected).toBe("true");
    expect(box().getAttribute("aria-describedby")).toBeTruthy();
    looksOn(other("m"));
    looksOn(screen.getByRole("button", { name: "A" }));
  });
});
