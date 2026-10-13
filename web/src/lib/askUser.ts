import type { AskKind, AskOption, AskQuestion } from "../api/types";

/** Normalize wire questions; legacy top-level options become a single q0.
 *  `free_text` is not consulted: every ask_user question ends in a fixed
 *  last-item input on the card. */
export function normalizeAskQuestions(
  kind: AskKind | undefined,
  summary: string,
  questions: AskQuestion[] | undefined,
  options: AskOption[] | undefined,
  multiSelect: boolean | undefined,
): AskQuestion[] {
  if (questions && questions.length > 0) return questions;
  if (kind === "ask_user" && options && options.length > 0) {
    return [
      {
        id: "q0",
        prompt: summary,
        options,
        multi_select: multiSelect,
      },
    ];
  }
  return [];
}
