import { CaretDown, CaretUp } from "@phosphor-icons/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { AskAnswer, AskOption, AskQuestion } from "../api/types";
import { normalizeAskQuestions } from "../lib/askUser";
import {
  ASK_FIELD,
  ASK_FLAT_BUTTON,
  ASK_STEP_BUTTON,
  AskCard,
  type Grant,
} from "./AskCard";
import { glyphFor } from "./ToolIcon";

/** One wheel notch moves on by exactly one question. The lock absorbs trackpad
 *  inertia: one gesture, however many events it fires, is one step. */
const STEP_LOCK_MS = 260;

/* ── 3/3 · ask_user: one question at a time → Submit / Cancel ──────────
   The pane renders the question in view and nothing else — one item container
   per question, as tall as that question, so there is no window to measure and
   nothing to clip. Three ways to move on, one step each: the wheel, the arrows in
   the title row (which also carry `current / total`), and — for a single choice -
   the pick itself, because there the click *is* the answer. The pane slides a
   hair in the direction of travel. The actions sit below the pane: the free text
   belongs to the question in view (the wire keeps free text per question), the
   buttons to the whole ask. Submit waits for an answer to every question; Cancel
   answers nothing. Its own module, because it is the one ask that carries a state
   machine — answers per question plus the legacy single-question compat — where
   the other two carry a receipt. */
export function AskUserCard({
  tool,
  summary,
  options,
  multiSelect,
  freeText,
  questions: questionsProp,
  onGrant,
}: {
  tool: string;
  summary: string;
  options: AskOption[];
  multiSelect: boolean;
  freeText: boolean;
  questions?: AskQuestion[];
  onGrant: Grant;
}) {
  const questions = useMemo(
    () =>
      normalizeAskQuestions(
        "ask_user",
        summary,
        questionsProp,
        options,
        multiSelect,
        freeText,
      ),
    [summary, questionsProp, options, multiSelect, freeText],
  );

  const [pickedByQ, setPickedByQ] = useState<Record<string, string[]>>({});
  const [textByQ, setTextByQ] = useState<Record<string, string>>({});
  /** Which question the pane shows, clamped — a fresh, shorter ask can never
   *  step past its end. `dir` only says which way the next one slides in. */
  const [step, setStep] = useState(0);
  const [dir, setDir] = useState<1 | -1>(1);
  const paneRef = useRef<HTMLDivElement>(null);
  /** True while a step settles: however many events a gesture fires, it is one. */
  const steppingRef = useRef(false);

  const multi = questions.length > 1;
  const index = Math.max(0, Math.min(step, questions.length - 1));
  const active: AskQuestion | undefined = questions[index];
  const qMulti = Boolean(active?.multi_select);
  const picked = active ? (pickedByQ[active.id] ?? []) : [];

  /** Move the pane on by one question, sliding in the direction of travel. Out of
   *  range is a no-op, so callers need no bounds check of their own. */
  const goTo = useCallback(
    (next: number) => {
      if (next < 0 || next >= questions.length || next === index) return;
      setDir(next > index ? 1 : -1);
      setStep(next);
    },
    [index, questions.length],
  );

  const toggleOption = (qid: string, oid: string, multi: boolean) => {
    setPickedByQ((prev) => {
      const cur = prev[qid] ?? [];
      const next = multi
        ? cur.includes(oid)
          ? cur.filter((x) => x !== oid)
          : [...cur, oid]
        : cur.includes(oid)
          ? []
          : [oid];
      return { ...prev, [qid]: next };
    });
  };

  /** An option click. For a single choice the pick *is* the answer, so the next
   *  question comes up on its own; a multi-select stays put (more may follow), as
   *  does the last question (nowhere to go) — and so does un-picking a choice,
   *  which leaves the question unanswered on purpose. */
  const pick = (oid: string) => {
    if (!active) return;
    const wasPicked = picked.includes(oid);
    toggleOption(active.id, oid, qMulti);
    if (!qMulti && !wasPicked) goTo(index + 1);
  };

  /** Wheel over the pane: one notch, one question — down for the next, up for the
   *  previous. The browser's own scrolling is suppressed: a gesture is a step,
   *  not a scroll. Answers stay keyed by question, so stepping back is free. */
  useEffect(() => {
    const pane = paneRef.current;
    if (!pane || !multi) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.deltaY) return;
      e.preventDefault();
      if (steppingRef.current) return;
      steppingRef.current = true;
      window.setTimeout(() => {
        steppingRef.current = false;
      }, STEP_LOCK_MS);
      goTo(index + (e.deltaY > 0 ? 1 : -1));
    };
    pane.addEventListener("wheel", onWheel, { passive: false });
    return () => pane.removeEventListener("wheel", onWheel);
  }, [goTo, multi, questions, index]);

  const allAnswered =
    questions.length > 0 &&
    questions.every((q) => (pickedByQ[q.id] ?? []).length > 0);

  const buildAnswers = (): Record<string, AskAnswer> => {
    const answers: Record<string, AskAnswer> = {};
    for (const q of questions) {
      const selected = pickedByQ[q.id] ?? [];
      const ft = (textByQ[q.id] ?? "").trim();
      answers[q.id] = {
        selected,
        ...(q.free_text && ft ? { free_text: ft } : {}),
      };
    }
    return answers;
  };

  const submit = (approved: boolean) => {
    if (!approved) {
      onGrant(false, false, {});
      return;
    }
    const answers = buildAnswers();
    const flat =
      questions.length === 1 ? answers[questions[0].id]?.selected : undefined;
    onGrant(true, false, {
      answers,
      ...(flat && flat.length > 0 ? { selected: flat } : {}),
    });
  };

  const showIntro =
    questions.length > 0 &&
    (multi || questions[0].prompt !== summary) &&
    Boolean(summary.trim());

  return (
    <AskCard
      kind="ask_user"
      icon={glyphFor(tool)}
      title={multi ? "The agent has questions" : "The agent has a question"}
      trailing={
        multi ? (
          <div className="mt-0.5 flex shrink-0 items-center gap-1.5">
            <span
              className="font-mono text-[11px] leading-none whitespace-nowrap text-(--_dk-text-muted)"
              data-testid="ask-user-count"
            >
              {index + 1} / {questions.length}
            </span>
            <button
              type="button"
              aria-label="Previous question"
              disabled={index === 0}
              onClick={() => goTo(index - 1)}
              className={ASK_STEP_BUTTON}
            >
              <CaretUp size={11} weight="bold" />
            </button>
            <button
              type="button"
              aria-label="Next question"
              disabled={index === questions.length - 1}
              onClick={() => goTo(index + 1)}
              className={ASK_STEP_BUTTON}
            >
              <CaretDown size={11} weight="bold" />
            </button>
          </div>
        ) : undefined
      }
      actions={
        <>
          <button
            type="button"
            disabled={!allAnswered}
            onClick={() => submit(true)}
            className={ASK_FLAT_BUTTON}
          >
            Submit
          </button>
          <button
            type="button"
            onClick={() => submit(false)}
            className={ASK_FLAT_BUTTON}
          >
            Cancel
          </button>
          {active?.free_text && (
            <input
              type="text"
              data-testid={`ask-user-free-text-${active.id}`}
              aria-label={`Free text for ${active.prompt || active.id} (optional)`}
              placeholder="Free text (optional)"
              className={`${ASK_FIELD} min-w-[8rem] flex-1`}
              value={textByQ[active.id] ?? ""}
              onChange={(e) =>
                setTextByQ((prev) => ({ ...prev, [active.id]: e.target.value }))
              }
            />
          )}
        </>
      }
    >
      {showIntro && (
        <p
          className="mt-1.5 text-dk-base text-(--_dk-text-body)"
          data-testid="ask-user-intro"
        >
          {summary}
        </p>
      )}
      {!multi && !showIntro && (
        <p className="mt-1.5 text-dk-base text-(--_dk-text-body)">
          {questions[0].prompt || summary}
        </p>
      )}

      {active && (
        <div ref={paneRef} className="mt-3" data-testid="ask-user-questions">
          {/* Keyed by question: a step remounts the pane, so the slide replays. */}
          <div
            key={active.id}
            data-step={dir > 0 ? "down" : "up"}
            data-testid={`ask-user-question-${active.id}`}
            data-question-id={active.id}
            className="permission-card__question flex flex-col gap-2"
          >
            {multi && (
              <p className="min-w-0 text-dk-base font-medium text-(--_dk-text-primary)">
                {active.prompt}
              </p>
            )}
            <div
              className="flex flex-col gap-2"
              role={qMulti ? "group" : "radiogroup"}
              aria-label={active.prompt || "Options"}
              data-testid={
                multi ? `ask-user-options-${active.id}` : "ask-user-options"
              }
            >
              {active.options.map((opt) => {
                const pickedOn = picked.includes(opt.id);
                return (
                  <button
                    key={opt.id}
                    type="button"
                    aria-pressed={pickedOn}
                    data-option-id={opt.id}
                    onClick={() => pick(opt.id)}
                    className={`${pickedOn ? "btn-primary" : "btn-ghost"} btn-sm w-full justify-start`}
                  >
                    {opt.label}
                  </button>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </AskCard>
  );
}
