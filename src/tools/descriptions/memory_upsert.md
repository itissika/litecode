Add or update small, durable session memory entries. This memory survives compaction and is shown to you again after a compaction; it is not a transcript or a long-term store.

Write only:
- explicit user preferences or corrections ("use pnpm", "don't touch X")
- decisions already made, with the reason if it matters
- hard constraints / environment quirks that change how you work
- unresolved key questions that must survive compaction

Do not write:
- task progress or step lists -> `todo`
- code structure or where things live -> `code_search`
- repo conventions/architecture -> `knowledge` / `AGENTS.md`
- raw history or quotes -> `session_search`
- one-off command output, secrets, or credentials

Rules:
- `text` is trimmed; at most 200 characters. Total across entries at most 4000 characters; at most 32 entries.
- Omit `id` to add. If an entry with the same text already exists, it is updated instead of duplicated.
- Pass `id` to update an existing entry; an unknown id is rejected, never created.
- `kind` is `fact` (default), `preference`, `decision`, or `constraint`.
- A batch is atomic per item: rejected items are reported, and the rest still apply.
- Keep it small. Delete stale entries with `memory.delete`; merge variants instead of appending them.
