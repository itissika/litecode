import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PermissionCard } from "./PermissionModal";

afterEach(cleanup);

/** A plan ask, as the plan tool sends it: tool `plan`, rule `create`, the plan
 *  summary as the reason, and free text enabled. */
const planAsk = {
  tool: "plan",
  ruleId: "create",
  summary: "把权限卡收敛成一句话 + 正文理由",
  kind: "approval" as const,
  freeText: true,
};

describe("PermissionCard — plan ask", () => {
  it("asks in one sentence and drops the grant bookkeeping", () => {
    render(<PermissionCard {...planAsk} onGrant={vi.fn()} />);

    // The sentence is the heading — no separate "Approve plan" line above it.
    expect(
      screen.getByRole("heading", { name: "The agent wants to create a plan" }),
    ).toBeTruthy();
    // No rule / request metadata: a plan ask carries no grant behind it.
    expect(screen.queryByText(/Rule:/)).toBeNull();
    expect(screen.queryByText(/Request:/)).toBeNull();
  });

  it("sets the reason as the card's prose body", () => {
    render(<PermissionCard {...planAsk} onGrant={vi.fn()} />);

    const reason = screen.getByText(planAsk.summary);
    expect(reason.tagName).toBe("P");
    // Chat prose: body size + body tone, not the old 11px muted caption.
    expect(reason.className).toContain("text-dk-base");
    expect(reason.className).toContain("--_dk-text-body");
    expect(reason.className).not.toContain("text-dk-xs");
  });

  it("keeps the opinion single-line, in the buttons' own row and after them", () => {
    render(<PermissionCard {...planAsk} onGrant={vi.fn()} />);

    const opinion = screen.getByLabelText("Opinion (optional)");
    expect(opinion.tagName).toBe("INPUT");
    expect(opinion.getAttribute("type")).toBe("text");

    const approve = screen.getByRole("button", { name: "Approve" });
    const reject = screen.getByRole("button", { name: "Reject" });
    // Same row as the decision, and after both buttons in document order.
    expect(opinion.parentElement).toBe(approve.parentElement);
    expect(opinion.parentElement).toBe(reject.parentElement);
    for (const button of [approve, reject]) {
      expect(
        button.compareDocumentPosition(opinion) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    }
  });

  it("carries a typed opinion into the receipt", async () => {
    const user = userEvent.setup();
    const onGrant = vi.fn();
    render(<PermissionCard {...planAsk} onGrant={onGrant} />);

    await user.type(screen.getByLabelText("Opinion (optional)"), "looks good");
    await user.click(screen.getByRole("button", { name: "Approve" }));

    expect(onGrant).toHaveBeenCalledWith(true, false, { freeText: "looks good" });
  });
});

describe("PermissionCard — tool ask", () => {
  const ask = { tool: "bash", ruleId: "default", summary: "npm run build" };

  it("names the tool and keeps the three grant buttons, no rule / request footer", () => {
    render(<PermissionCard {...ask} onGrant={vi.fn()} />);

    expect(screen.getByRole("heading", { name: "bash" })).toBeTruthy();
    expect(screen.getByText("npm run build")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Allow once" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Always allow" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Deny" })).toBeTruthy();
    // The catch-all rule names no boundary: the card translates nothing.
    expect(screen.queryByText("Outside the workspace")).toBeNull();
    expect(screen.queryByText("Risky command")).toBeNull();
    expect(screen.queryByText(/Rule:/)).toBeNull();
    // No free text on a tool ask.
    expect(screen.queryByLabelText("Opinion (optional)")).toBeNull();
  });

  it("translates the boundary the call ran into", () => {
    const { unmount } = render(
      <PermissionCard
        tool="write"
        ruleId="outside_workspace"
        summary="../notes/draft.md"
        onGrant={vi.fn()}
      />,
    );
    expect(screen.getByText("Outside the workspace")).toBeTruthy();
    unmount();

    render(
      <PermissionCard
        tool="bash"
        ruleId="custom-rule"
        summary="sudo rm -rf /var/tmp/build"
        onGrant={vi.fn()}
      />,
    );
    expect(screen.getByText("Risky command")).toBeTruthy();
  });

  it("keeps the call on one line, tails a ! and shows the whole text on hover", () => {
    const scroll = vi
      .spyOn(Element.prototype, "scrollWidth", "get")
      .mockReturnValue(900);
    const client = vi
      .spyOn(Element.prototype, "clientWidth", "get")
      .mockReturnValue(200);

    render(
      <PermissionCard
        {...ask}
        summary={`echo ${"x".repeat(200)}`}
        onGrant={vi.fn()}
      />,
    );

    const line = screen.getByText(/^echo /);
    expect(line.className).toContain("truncate");
    expect(line.closest("p")?.getAttribute("title")).toBe(line.textContent);
    expect(screen.getByText("!")).toBeTruthy();

    scroll.mockRestore();
    client.mockRestore();
  });

  it("dresses the three decisions like the chat input's own buttons", () => {
    render(<PermissionCard {...ask} onGrant={vi.fn()} />);

    for (const name of ["Allow once", "Always allow", "Deny"]) {
      const cls = screen.getByRole("button", { name }).className;
      expect(cls).toContain("backdrop-blur-[12px]");
      expect(cls).toContain("border-(--_dk-border-strong)");
      expect(cls).toContain("h-[30px]");
      expect(cls).toContain("active:scale-90");
      expect(cls).not.toContain("btn-primary");
    }
  });
});

describe("PermissionCard — ask_user", () => {
  const ask = {
    tool: "ask_user",
    ruleId: "ask",
    summary: "Which approach should we take?",
    kind: "ask_user" as const,
    freeText: true,
    options: [
      { id: "a", label: "Option A" },
      { id: "b", label: "Option B" },
    ],
    multiSelect: false,
  };

  it("shows question, options, Submit/Cancel — not Allow/Approve", () => {
    render(<PermissionCard {...ask} onGrant={vi.fn()} />);
    expect(
      screen.getByRole("heading", { name: "The agent has a question" }),
    ).toBeTruthy();
    expect(screen.getByText(ask.summary)).toBeTruthy();
    expect(screen.getByTestId("ask-user-options")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Submit" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
    expect(screen.queryByText(/Rule:/)).toBeNull();
  });

  it("disables Submit until an option is picked, then sends answers map", async () => {
    const user = userEvent.setup();
    const onGrant = vi.fn();
    render(<PermissionCard {...ask} onGrant={onGrant} />);
    const submit = screen.getByRole("button", { name: "Submit" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    await user.click(screen.getByRole("button", { name: "Option A" }));
    expect(submit.disabled).toBe(false);
    await user.type(
      screen.getByTestId("ask-user-free-text-q0"),
      "more detail",
    );
    await user.click(submit);
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: {
        q0: { selected: ["a"], free_text: "more detail" },
      },
      selected: ["a"],
    });
  });

  it("shows one question at a time, count in the title row, free text below", () => {
    render(
      <PermissionCard
        tool="ask_user"
        ruleId="ask"
        summary=""
        kind="ask_user"
        questions={[
          {
            id: "one",
            prompt: "First?",
            options: [{ id: "a", label: "A" }],
            free_text: true,
          },
          {
            id: "two",
            prompt: "Second?",
            options: [{ id: "b", label: "B" }],
            free_text: true,
          },
        ]}
        onGrant={vi.fn()}
      />,
    );

    // The pane holds one question container — the question in view and nothing
    // else, so its height is that question's height: no scroll window, no clip.
    const pane = screen.getByTestId("ask-user-questions");
    expect(pane.children.length).toBe(1);
    expect(pane.className).not.toContain("overflow");
    expect(screen.getByTestId("ask-user-question-one")).toBeTruthy();
    expect(screen.queryByTestId("ask-user-question-two")).toBeNull();
    // The title row says where we are and carries the way up and down.
    expect(screen.getByTestId("ask-user-count").textContent).toBe("1 / 2");
    expect(
      (
        screen.getByRole("button", {
          name: "Previous question",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "Next question",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);

    // Choices stack one per row — full width, label to the left.
    expect(screen.getByTestId("ask-user-options-one").className).toContain(
      "flex-col",
    );
    const optionA = screen.getByRole("button", { name: "A" });
    expect(optionA.className).toContain("w-full");
    expect(optionA.className).toContain("justify-start");

    // The card's own controls carry no fill of their own: the hairline marks them
    // and hover only washes them — the card's glass stays the only surface.
    for (const name of [
      "Submit",
      "Cancel",
      "Previous question",
      "Next question",
    ]) {
      const cls = screen.getByRole("button", { name }).className;
      expect(cls).not.toContain("backdrop-blur");
      expect(cls).not.toContain("color-mix");
      expect(cls).toContain("border-(--_dk-border-strong)");
      expect(cls).toContain("hover:bg-(--_dk-ix-bg-hover)");
    }

    // Both questions allow free text; the footer owns the one in view and stays
    // outside the pane, in the buttons' own row.
    const free = screen.getByTestId("ask-user-free-text-one");
    expect(pane.contains(free)).toBe(false);
    expect(free.parentElement).toBe(
      screen.getByRole("button", { name: "Submit" }).parentElement,
    );
  });

  it("steps one question per wheel notch — down and up — footer in tow", async () => {
    const step = (deltaY: number) =>
      fireEvent.wheel(screen.getByTestId("ask-user-questions"), { deltaY });
    const shown = () =>
      (screen.getByTestId("ask-user-questions").firstElementChild as HTMLElement)
        .dataset;

    render(
      <PermissionCard
        tool="ask_user"
        ruleId="ask"
        summary=""
        kind="ask_user"
        questions={["one", "two", "three"].map((id, i) => ({
          id,
          prompt: `Q${i + 1}?`,
          options: [{ id: `o${i}`, label: `O${i}` }],
          free_text: true,
        }))}
        onGrant={vi.fn()}
      />,
    );

    expect(shown().questionId).toBe("one");

    // One notch down: the next question replaces it, sliding in from below.
    step(120);
    expect(shown().questionId).toBe("two");
    expect(shown().step).toBe("down");
    expect(screen.getByTestId("ask-user-count").textContent).toBe("2 / 3");
    expect(screen.getByTestId("ask-user-free-text-two")).toBeTruthy();

    // One gesture is one step: the lock swallows the rest of a burst.
    step(120);
    step(120);
    expect(shown().questionId).toBe("two");

    // Settled, the next notch steps on — and up goes back, sliding in from above.
    await new Promise((r) => setTimeout(r, 300));
    step(120);
    expect(shown().questionId).toBe("three");
    await new Promise((r) => setTimeout(r, 300));
    step(-120);
    expect(shown().questionId).toBe("two");
    expect(shown().step).toBe("up");
  });

  it("steps from the title row's arrows, counting along", async () => {
    const user = userEvent.setup();
    render(
      <PermissionCard
        tool="ask_user"
        ruleId="ask"
        summary=""
        kind="ask_user"
        questions={["one", "two", "three"].map((id, i) => ({
          id,
          prompt: `Q${i + 1}?`,
          options: [{ id: `o${i}`, label: `O${i}` }],
        }))}
        onGrant={vi.fn()}
      />,
    );

    const shown = () =>
      (screen.getByTestId("ask-user-questions").firstElementChild as HTMLElement)
        .dataset.questionId;
    const count = () => screen.getByTestId("ask-user-count").textContent;
    const prev = () =>
      screen.getByRole("button", {
        name: "Previous question",
      }) as HTMLButtonElement;
    const next = () =>
      screen.getByRole("button", { name: "Next question" }) as HTMLButtonElement;

    expect(count()).toBe("1 / 3");
    expect(prev().disabled).toBe(true);

    await user.click(next());
    expect(shown()).toBe("two");
    expect(count()).toBe("2 / 3");

    await user.click(next());
    expect(shown()).toBe("three");
    expect(count()).toBe("3 / 3");
    expect(next().disabled).toBe(true);

    await user.click(prev());
    expect(shown()).toBe("two");
    expect(count()).toBe("2 / 3");
  });

  it("multi-select stays put — more than one may follow", async () => {
    const user = userEvent.setup();
    render(
      <PermissionCard
        tool="ask_user"
        ruleId="ask"
        summary=""
        kind="ask_user"
        questions={[
          {
            id: "one",
            prompt: "Pick some",
            options: [
              { id: "a", label: "A" },
              { id: "b", label: "B" },
            ],
            multi_select: true,
          },
          { id: "two", prompt: "Q2?", options: [{ id: "c", label: "C" }] },
        ]}
        onGrant={vi.fn()}
      />,
    );

    const shown = () =>
      (screen.getByTestId("ask-user-questions").firstElementChild as HTMLElement)
        .dataset.questionId;

    await user.click(screen.getByRole("button", { name: "A" }));
    await user.click(screen.getByRole("button", { name: "B" }));
    expect(shown()).toBe("one");
    expect(screen.getByTestId("ask-user-count").textContent).toBe("1 / 2");
  });

  it("Cancel sends approved false without requiring a pick", async () => {
    const user = userEvent.setup();
    const onGrant = vi.fn();
    render(<PermissionCard {...ask} onGrant={onGrant} />);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onGrant).toHaveBeenCalledWith(false, false, {});
  });

  it("multi_select allows several options", async () => {
    const user = userEvent.setup();
    const onGrant = vi.fn();
    render(
      <PermissionCard {...ask} multiSelect freeText={false} onGrant={onGrant} />,
    );
    await user.click(screen.getByRole("button", { name: "Option A" }));
    await user.click(screen.getByRole("button", { name: "Option B" }));
    await user.click(screen.getByRole("button", { name: "Submit" }));
    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: { q0: { selected: ["a", "b"] } },
      selected: ["a", "b"],
    });
  });

  it("lists multiple questions and submits answers keyed by id", async () => {
    const user = userEvent.setup();
    const onGrant = vi.fn();
    render(
      <PermissionCard
        tool="ask_user"
        ruleId="ask"
        summary="A few things to confirm"
        kind="ask_user"
        questions={[
          {
            id: "tone",
            prompt: "Preferred tone?",
            options: [
              { id: "formal", label: "Formal" },
              { id: "casual", label: "Casual" },
            ],
          },
          {
            id: "depth",
            prompt: "How deep?",
            options: [
              { id: "brief", label: "Brief" },
              { id: "deep", label: "Deep" },
            ],
            multi_select: true,
            free_text: true,
          },
        ]}
        onGrant={onGrant}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "The agent has questions" }),
    ).toBeTruthy();
    expect(screen.getByTestId("ask-user-intro").textContent).toBe(
      "A few things to confirm",
    );
    expect(screen.getByTestId("ask-user-question-tone")).toBeTruthy();
    // One at a time: the second question is not in the tree yet.
    expect(screen.queryByTestId("ask-user-question-depth")).toBeNull();

    const submit = screen.getByRole("button", { name: "Submit" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    // A single choice answers and moves on by itself — no wheel needed.
    await user.click(screen.getByRole("button", { name: "Formal" }));
    expect(screen.getByTestId("ask-user-question-depth")).toBeTruthy();
    expect(screen.queryByTestId("ask-user-question-tone")).toBeNull();
    expect(submit.disabled).toBe(true);

    await user.click(screen.getByRole("button", { name: "Brief" }));
    await user.click(screen.getByRole("button", { name: "Deep" }));
    expect(submit.disabled).toBe(false);

    // The free text belongs to the question in view; what the first question was
    // answered before stepping away is still there.
    await user.type(
      screen.getByTestId("ask-user-free-text-depth"),
      "go further on APIs",
    );
    await user.click(submit);

    expect(onGrant).toHaveBeenCalledWith(true, false, {
      answers: {
        tone: { selected: ["formal"] },
        depth: {
          selected: ["brief", "deep"],
          free_text: "go further on APIs",
        },
      },
    });
  });
});
