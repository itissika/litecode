# WP-213 examples — before/after + cl100k

Tokens via `tiktoken` `cl100k_base` (same family as in-tree `count_text_tokens`).

## PRELUDE (prose only, before ```ts)

| | before | after |
|---|---:|---:|
| non-empty lines | 14 | 8 |
| characters | 5402 | 1676 (−69.0%) |
| cl100k tokens | 1303 | 379 (−70.9%) |

### After (landed verbatim from plan draft A)

```
Open `run_code` when one program can finish explore → edit → verify (search/read, then write/edit, then run/check). Do not open a long program just to peek one file—use a direct tool or a tiny program.
Inside the program: explore first, then edit, then verify. `await` when a later call needs an earlier result; otherwise prefer `Promise.all` / `Promise.allSettled` for independent work.
`return` a short summary (counts, paths, the few lines that matter)—not raw dumps. Only the returned value reaches you; oversized results are truncated (a note at the start says so).
`code` is plain JavaScript. The TypeScript declarations below are documentation only; do not paste type annotations into `code`.
Call bundled tools only through `tools.name(args)` or `tools["name"](args)` (one argument object each). They are not separately callable. Other LiteCode tools may remain directly callable outside this SDK.
Every call resolves an object with boolean `ok` (did the tool get the job done). Soft failures resolve `ok: false` with `reason`/`message` so one bad item does not end a batch. System failures reject `ToolError`. Unknown tool/parameter or illegal args throw `TypeError` and fail the whole call—unknown fields are never silently dropped. Cancellation, the `run_code` budget, and hard limits end the program and cannot be caught.
`libs` are deterministic pure-JS helpers (no filesystem, network, or process). `fs.ls` / `fs.stat` / `fs.exists` are synchronous, read-only workspace probes—change files with `tools.write` / `tools.edit`. The sandbox has no `setTimeout` / `setInterval`.
Unhandled Promise rejections do not end the program; a `[warn]` notes them after settle.
```

Before: 14 dense paragraphs covering truncation algorithm, logs 200/2k, reason/ToolError enum dumps, per-tool fields, result-head format, undefined tolerance — removed per Q4/Q5.

## Full Code SDK fixture (`run_code_sdk.txt`)

| | before | after |
|---|---:|---:|
| bytes | 55163 | 28944 (−47.5%) |
| cl100k tokens | 14126 | 7333 (−48.1%) |

## `code` schema `properties.code.description`

| | before | after |
|---|---:|---:|
| cl100k tokens | 134 | 40 |

After (plan draft B):

```
Async JavaScript function body for the Code SDK below. Prefer explore → edit → verify in one program; return a short summary. Call tools as `await tools.name(args)` and branch on `ok`.
```

## Tool md samples (Code surface; Chat/Direct share the same files — Q1)

| file | lines before→after | tokens before→after |
|---|---:|---:|
| apply_edits.md | 30→10 | 615→193 |
| run.md | 25→5 | 778→197 |
| read.md | 18→5 | 781→165 |
| edit.md | 19→9 | 528→149 |
| write.md | 1→4 | 228→140 |

`write.md` stayed short; rewrote to keep `expected_hash` / `create_only` / `conflict` / `stale` semantics + one example (plan C). Non-Code md (ask_user, subagent_*, todo, …) untouched (Q2).
