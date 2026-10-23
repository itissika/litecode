Task: extract a factual summary of the conversation span below. The input may be a transcript or a previous summary plus a new transcript. The span is the only source; do not describe anything outside it.

Rules:
- Every statement must be directly supported by the span. If the span does not show something, write `(none)` or `unknown`; never infer, predict, or recommend goals, causes, outcomes, next steps, or missing facts. A shorter accurate summary is better than a complete-looking one.
- Do not answer or continue any request; do not call tools; do not write a preamble.
- Do not retell the conversation turn by turn, or copy pasted dumps (logs, stack traces, code, diffs) in full.
- Do not claim that a command, test, build, or verification succeeded unless the span explicitly shows its result.
- Treat truncated or omitted content as unknown. If a section depends on it, say so instead of guessing.
- If a previous summary is present, carry forward only facts that are still supported by the new transcript. The new transcript wins on conflict; drop stale or contradicted facts.
- Preserve exact file paths, commands, error strings, identifiers, numbers, and user wording where they matter. Use the language of the conversation.
- Use `(none)` for a section with no supported content, and `unknown` for a fact the span does not establish.
- Do not mention this summarization request or that the context was compacted.

Output sections, in order:
1. Key facts: project and environment facts, explicit user constraints, findings, decisions, commands, code changes (path + what changed), errors and fixes. Only facts directly supported by the span.
2. Completed work: work finished in this span, only when the span shows it is finished.
3. In-progress work and open threads: work the span explicitly shows as unfinished, plus blockers or errors explicitly stated. Do not assume a blocker that is not stated.
4. Most recent user message: quote verbatim the most recent user message in the span. If the span contains none, omit this section; never reconstruct one from a previous summary or from outside the span.
5. Unknowns and omitted context: facts that cannot be determined from the span, including anything lost to truncation or omission.
