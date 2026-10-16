import { CaretDown, CaretUp } from "@phosphor-icons/react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import type { AskAnswer, AskOption, AskQuestion } from "../api/types";
import { normalizeAskQuestions } from "../lib/askUser";
import {
  ASK_FIELD,
  ASK_FLAT_BUTTON,
  ASK_STEP_BUTTON,
  AskCard,
  type Grant,
} from "./AskCard";
import { compactMarkdownComponents } from "./compactMarkdown";
import { glyphFor } from "./ToolIcon";

/** One wheel notch moves on by exactly one question. The lock absorbs trackpad
 *  inertia: one gesture, however many events it fires, is one step. */
const STEP_LOCK_MS = 260;

/** Label of the fixed last item every question ends in. */
export const OTHER_LABEL = "其他（自己填写）";

/** The option's own seat in its question — a, b, c … so a list can be told apart
 *  at a glance, one square per option. Past z a list of 27+ falls back to
 *  numbers; nothing else changes. */
function optionMark(i: number): string {
  return i < 26 ? String.fromCharCode(97 + i) : String(i + 1);
}

/* ── The ask's own prose, in markdown ────────────────────────────────
   The agent writes the question, so the question may carry markdown: bold,
   inline code, a list, a link. Compact on purpose — a dock card has no room for
   `AgentMarkdown`'s shiki, mermaid and copy buttons, and the toast's renderer
   stops at inline. Type comes from the caller's wrapper (which also owns the
   wrap-anywhere rule); every mapping here is spacing and inline treatment. */
function AskProse({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        ...compactMarkdownComponents,
        p: ({ children }: { children?: ReactNode }) => (
          <p className="m-0 leading-snug [&+p]:mt-1.5">{children}</p>
        ),
        ul: ({ children }: { children?: ReactNode }) => (
          <ul className="m-0 mt-1.5 list-disc pl-5 first:mt-0">{children}</ul>
        ),
        ol: ({ children }: { children?: ReactNode }) => (
          <ol className="m-0 mt-1.5 list-decimal pl-5 first:mt-0">{children}</ol>
        ),
        li: ({ children }: { children?: ReactNode }) => (
          <li className="marker:text-(--_dk-text-muted) [&+li]:mt-0.5">
            {children}
          </li>
        ),
        pre: ({ children }: { children?: ReactNode }) => (
          <pre className="m-0 mt-1.5 overflow-x-auto rounded-sm bg-(--_dk-side) p-2 font-mono text-[12px] leading-snug first:mt-0 [&>code]:bg-transparent [&>code]:p-0">
            {children}
          </pre>
        ),
      }}
    >
      {text}
    </ReactMarkdown>
  );
}

/* ── 3/3 · ask_user: one question at a time → Submit / Cancel ──────────
   The pane renders the question in view and nothing else — one item container
   per question, as tall as that question, so there is no window to measure and
   nothing to clip. Three ways to move on, one step each: the wheel, the arrows in
   the title row (which also carry `current / total`), and — for a single choice -
   the pick itself, because there the click *is* the answer. The pane slides a
   hair in the direction of travel. Every question ends in a fixed inline input,
   「其他（自己填写）」 — not a protocol option (no option id, never in `selected`),
   always shown whatever `free_text` says. Single choice: the input and the options
   exclude each other (typing clears the pick; a pick drops the text from the
   answer). Multi-select: they coexist. Text alone answers a question; text is
   trimmed, and blank means unanswered. The buttons below the pane belong to the
   whole ask. Submit waits for an answer to every question; Cancel
   answers nothing. Its own module, because it is the one ask that carries a state
   machine — answers per question plus the legacy single-question compat — where
   the other two carry a receipt. */
export function AskUserCard({
  tool,
  summary,
  options,
  multiSelect,
  questions: questionsProp,
  onGrant,
}: {
  tool: string;
  summary: string;
  options: AskOption[];
  multiSelect: boolean;
  /** Ignored (deprecated): the last-item input is always shown. Kept optional so
   *  callers that still pass it compile unchanged. */
  freeText?: boolean;
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
      ),
    [summary, questionsProp, options, multiSelect],
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

  /** Typing into a single choice's last item selects it: the options' pick clears. */
  const typeFree = (qid: string, value: string, multi: boolean) => {
    setTextByQ((prev) => ({ ...prev, [qid]: value }));
    if (!multi && value.trim()) {
      setPickedByQ((prev) => ({ ...prev, [qid]: [] }));
    }
  };

  /** The trimmed text that counts for a question: none when blank, and none for a
   *  single choice whose options hold the pick (the two exclude each other). */
  const freeFor = (q: AskQuestion): string => {
    const ft = (textByQ[q.id] ?? "").trim();
    if (!ft) return "";
    if (!q.multi_select && (pickedByQ[q.id] ?? []).length > 0) return "";
    return ft;
  };

  const allAnswered =
    questions.length > 0 &&
    questions.every(
      (q) => (pickedByQ[q.id] ?? []).length > 0 || freeFor(q) !== "",
    );

  const buildAnswers = (): Record<string, AskAnswer> => {
    const answers: Record<string, AskAnswer> = {};
    for (const q of questions) {
      const ft = freeFor(q);
      const selected = !q.multi_select && ft ? [] : (pickedByQ[q.id] ?? []);
      answers[q.id] = {
        selected,
        ...(ft ? { free_text: ft } : {}),
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

  /** Enter keeps the ask moving: the next question while there is one, the ask
   *  itself on the last. Only a question that counts as answered steps on, so a
   *  stray keypress can never skip a blank; and an IME's Enter (picking a
   *  candidate) belongs to the IME, never to the ask. */
  const keyFree = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter" || e.nativeEvent.isComposing || !active) return;
    const answered =
      (pickedByQ[active.id] ?? []).length > 0 || freeFor(active) !== "";
    if (!answered) return;
    e.preventDefault();
    if (index < questions.length - 1) goTo(index + 1);
    else if (allAnswered) submit(true);
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
        </>
      }
    >
      {showIntro && (
        <div
          className="mt-1.5 min-w-0 text-dk-base text-(--_dk-text-body) [overflow-wrap:anywhere]"
          data-testid="ask-user-intro"
        >
          <AskProse text={summary} />
        </div>
      )}
      {!multi && !showIntro && (
        <div className="mt-1.5 min-w-0 text-dk-base text-(--_dk-text-body) [overflow-wrap:anywhere]">
          <AskProse text={questions[0].prompt || summary} />
        </div>
      )}

      {active && (
        <div ref={paneRef} className="mt-3" data-testid="ask-user-questions">
          {/* Keyed by question: a step remounts the pane, so the slide replays. */}
          <div
            key={active.id}
            data-step={dir > 0 ? "down" : "up"}
            data-testid={`ask-user-question-${active.id}`}
            data-question-id={active.id}
            className="permission-card__question flex min-w-0 flex-col gap-2"
          >
            {multi && (
              <div className="min-w-0 text-dk-base font-medium text-(--_dk-text-primary) [overflow-wrap:anywhere]">
                <AskProse text={active.prompt} />
              </div>
            )}
            <div
              className="flex min-w-0 flex-col gap-1.5"
              role={qMulti ? "group" : "radiogroup"}
              aria-label={active.prompt || "Options"}
              data-testid={
                multi ? `ask-user-options-${active.id}` : "ask-user-options"
              }
            >
              {active.options.map((opt, i) => {
                const pickedOn = picked.includes(opt.id);
                return (
                  <button
                    key={opt.id}
                    type="button"
                    aria-pressed={pickedOn}
                    data-option-id={opt.id}
                    onClick={() => pick(opt.id)}
                    className={`permission-card__option ${pickedOn ? "btn-primary" : "btn-ghost"} btn-sm text-dk-base w-full`}
                  >
                    {/* Chrome, not content: the mark never joins the option's
                        accessible name (the label alone is the name). */}
                    <span aria-hidden className="permission-card__mark">
                      {optionMark(i)}
                    </span>
                    <span className="min-w-0 flex-1">{opt.label}</span>
                  </button>
                );
              })}
              {/* The last item is a bare field — no option chrome, no visible
                 label: the text *is* the answer, so the field speaks for itself
                 (the placeholder shows the words, `aria-label` names it). */}
              <input
                type="text"
                data-testid={`ask-user-free-text-${active.id}`}
                aria-label={OTHER_LABEL}
                placeholder={OTHER_LABEL}
                className={`${ASK_FIELD} w-full`}
                value={textByQ[active.id] ?? ""}
                onChange={(e) => typeFree(active.id, e.target.value, qMulti)}
                onKeyDown={keyFree}
              />
            </div>
          </div>
        </div>
      )}
    </AskCard>
  );
}
