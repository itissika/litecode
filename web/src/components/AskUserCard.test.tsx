import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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
      expect(input.parentElement?.lastElementChild).toBe(input);
      expect(input.hasAttribute("data-option-id")).toBe(false);
      await user.click(screen.getByRole("button", { name: "Next question" })).catch(() => {});
    }
  });

  it("2. label is Other (type your own)", () => {
    renderCard([single]);
    expect(OTHER_LABEL).toBe("Other (type your own)");
    const input = screen.getByRole("textbox", { name: "Other (type your own)" });
    expect(input.getAttribute("placeholder")).toBe("Other (type your own)");
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

/* WP-083 gave the last item a visible label and the options' own selected look;
   that dressing is gone again (user call): the row is a bare field, and the
   answer it carries is the only state it has. */
describe("AskUserCard last item: a bare field", () => {
  afterEach(cleanup);

  const box = () => screen.getByRole("textbox", { name: OTHER_LABEL });
  const pressed = (name: string) =>
    screen.getByRole("button", { name }).getAttribute("aria-pressed");

  it("a. the row is the field itself — no label, no option chrome", () => {
    renderCard([single]);
    const field = box();
    expect(field.tagName).toBe("INPUT");
    expect(screen.queryByText(OTHER_LABEL)).toBeNull(); // placeholder only
    expect(field.getAttribute("placeholder")).toBe(OTHER_LABEL);
    expect(field.className).toContain("permission-card__input");
    expect(field.className).not.toContain("btn-primary");
    expect(field.className).not.toContain("btn-ghost");
    expect(field.hasAttribute("data-selected")).toBe(false);
    expect(field.hasAttribute("aria-pressed")).toBe(false);
    expect(field.hasAttribute("aria-describedby")).toBe(false);
  });

  it("b. single: typing answers the question and clears the pick", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([single]);
    await user.click(screen.getByRole("button", { name: "A" }));
    await user.type(box(), "mine");
    expect(pressed("A")).toBe("false");
    expect(pressed("B")).toBe("false");
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { s: { selected: [], free_text: "mine" } },
    });
  });

  it("c. multi: the field and the picks coexist", async () => {
    const user = userEvent.setup();
    const { onGrant, submit } = renderCard([multi]);
    await user.click(screen.getByRole("button", { name: "A" }));
    await user.type(box(), "also");
    expect(pressed("A")).toBe("true");
    await user.click(submit());
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { m: { selected: ["a"], free_text: "also" } },
      selected: ["a"],
    });
  });
});

describe("AskUserCard prose: markdown", () => {
  afterEach(cleanup);

  it("renders the question's markdown — bold, code, list, emphasis", () => {
    renderCard([
      {
        id: "md",
        prompt: "Pick **one**:\n\n- `alpha` first\n- beta with _emphasis_",
        options: [{ id: "a", label: "A" }],
      },
    ]);
    expect(screen.getByText("one").tagName).toBe("STRONG");
    expect(screen.getByText("alpha").tagName).toBe("CODE");
    expect(
      screen.getAllByRole("listitem").map((li) => li.textContent),
    ).toEqual(["alpha first", "beta with emphasis"]);
    expect(screen.getByText("emphasis").tagName).toBe("EM");
    // Options stay plain text: no markdown dialect inside a button.
    expect(screen.getByRole("button", { name: "A" })).toBeTruthy();
  });

  it("renders the intro/summary markdown too", () => {
    render(
      <AskUserCard
        tool="ask_user"
        summary="Which **approach**?"
        options={[]}
        multiSelect={false}
        questions={[
          { id: "q", prompt: "Which one?", options: [{ id: "a", label: "A" }] },
        ]}
        onGrant={vi.fn()}
      />,
    );
    expect(
      screen.getByTestId("ask-user-intro").querySelector("strong")?.textContent,
    ).toBe("approach");
  });
});

describe("AskUserCard Enter key", () => {
  afterEach(cleanup);

  it("single: Enter answers the ask", async () => {
    const user = userEvent.setup();
    const { onGrant } = renderCard([single]);
    await user.type(screen.getByTestId("ask-user-free-text-s"), "mine{Enter}");
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { s: { selected: [], free_text: "mine" } },
    });
  });

  it("multi: Enter steps on and keeps the text; the last Enter sends", async () => {
    const user = userEvent.setup();
    const { onGrant } = renderCard([single, multi]);
    await user.type(screen.getByTestId("ask-user-free-text-s"), "x{Enter}");
    expect(onGrant).not.toHaveBeenCalled();
    expect(screen.getByTestId("ask-user-count").textContent).toBe("2 / 2");
    expect(screen.getByTestId("ask-user-question-m")).toBeTruthy();
    await user.type(screen.getByTestId("ask-user-free-text-m"), "y{Enter}");
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: {
        s: { selected: [], free_text: "x" },
        m: { selected: [], free_text: "y" },
      },
    });
  });

  it("blank: Enter neither steps on nor sends", () => {
    const { onGrant } = renderCard([single, multi]);
    fireEvent.keyDown(screen.getByTestId("ask-user-free-text-s"), {
      key: "Enter",
    });
    expect(onGrant).not.toHaveBeenCalled();
    expect(screen.getByTestId("ask-user-question-s")).toBeTruthy();
  });

  it("an IME's own Enter belongs to the IME", async () => {
    const user = userEvent.setup();
    const { onGrant } = renderCard([single, multi]);
    const box = screen.getByTestId("ask-user-free-text-s");
    await user.type(box, "x");
    fireEvent.keyDown(box, { key: "Enter", isComposing: true });
    expect(onGrant).not.toHaveBeenCalled();
    expect(screen.getByTestId("ask-user-question-s")).toBeTruthy();
  });
});

describe("AskUserCard option marks", () => {
  afterEach(cleanup);

  const marks = () =>
    Array.from(
      screen.getByTestId("ask-user-options").querySelectorAll("button"),
    ).map((b) => b.querySelector(".permission-card__mark")?.textContent ?? "");

  it("gives each option its own seat letter, out of the accessible name", () => {
    renderCard([
      {
        id: "many",
        prompt: "Pick one?",
        options: [
          { id: "a", label: "First" },
          { id: "b", label: "Second" },
          { id: "c", label: "Third" },
        ],
      },
    ]);
    expect(marks()).toEqual(["a", "b", "c"]);
    expect(screen.getByRole("button", { name: "First" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "a First" })).toBeNull();
  });

  it("numbers the seats past z", () => {
    renderCard([
      {
        id: "long",
        prompt: "Pick one?",
        options: Array.from({ length: 28 }, (_, i) => ({
          id: `o${i}`,
          label: `Option ${i}`,
        })),
      },
    ]);
    const seat = marks();
    expect(seat[0]).toBe("a");
    expect(seat[25]).toBe("z");
    expect(seat[26]).toBe("27");
    expect(seat[27]).toBe("28");
  });
});
