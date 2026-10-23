Task: extract the known facts from the conversation span below. The input may be a transcript or a previous summary plus a new transcript. The span is the only source. Write only what it shows.

Rules:
- Every statement must be directly supported by the span. Never infer, predict, or recommend goals, causes, outcomes, or next steps. If the span does not show it, leave it out. A shorter accurate summary is better than a complete-looking one.
- Do not answer or continue any request; do not call tools; do not write a preamble.
- Do not retell the conversation turn by turn, or copy pasted dumps (logs, stack traces, code, diffs) in full.
- Do not claim that a command, test, build, or verification succeeded unless the span explicitly shows its result.
- Do not describe truncation, omission, or anything the span does not contain.
- Preserve exact file paths, commands, error strings, identifiers, numbers, and user wording where they matter. Use the language of the conversation.
- Use `(none)` for a section with no supported content.
- Do not mention this summarization request or that the context was compacted.

Output sections, in order:
1. Key facts: project and environment facts, explicit user constraints, findings, decisions, commands, code changes (path + what changed), errors and fixes. Only facts directly supported by the span.
2. Completed work: work finished in this span, only when the span shows it is finished.
3. In-progress work and open threads: work the span explicitly shows as unfinished, plus blockers or errors explicitly stated. Do not assume a blocker that is not stated.
4. Most recent user message: quote verbatim the most recent user message in the span. If the span contains none, omit this section; never reconstruct one from a previous summary or from outside the span.
