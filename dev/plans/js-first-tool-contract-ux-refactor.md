# JS-first Code SDK v2 - API UX / contract consistency refactor

Status: accepted, implemented
Base branch: `refactor/js-first-tool-contract`
Current implementation commit: `945dcbd4` (`refactor: make run_code SDK JS-first and contract-driven`)

## 1. Background

The JS-first contract refactor is in place: `sdk/src/api.d.ts` owns built-in
code tool types, signatures, docs, and JSON Schema projection. Rust embeds the
generated contract and dynamically assembles only the runtime-visible tools.

Evaluation and review found that the contract is complete but not yet
conceptually clean enough for an agent consumer:

- suite membership is inconsistent: `memory.*` is namespaced while `file`,
  `process`, `web`, `browser`, and `lsp` are flat or prefixed;
- the prelude duplicates type-level information and reads like a tutorial;
- failure shape is split between `ToolFailure`, data-level `ok:false`, and
  unrelated status fields;
- `code_search` reports engine readiness as a successful data object with
  `ok:false`, instead of distinguishing not-searched from a real empty result;
- memory carries a `kind` classification and query filter that are not needed
  for its actual use: session memory is re-injected across compaction and the
  agent reads an authoritative snapshot to confirm before mutating entries;
- no-argument tools still require a `{}` argument;
- several field names and reasons are under-specified or inconsistent.

## 2. Goals

1. Make one suite rule for all built-in and custom code tools:
   `tools.<suite>.<member>(args)`.
2. Make one top-level success/failure rule:
   `ToolResult<T> = ToolSuccess<T> | ToolFailure`.
3. Make success payload types payload-only; the host/bridge owns `ok:true`.
4. Simplify `code_search`: executed search is success, including zero hits.
5. Simplify session memory: remove `kind`; make `memory.list` an atomic
   no-argument snapshot.
6. Allow no-argument tools to be called without `{}`.
7. Reduce the prelude to run_code philosophy plus runtime facts that types
   cannot express; move the rest next to the relevant types.
8. Keep Rust buildable without Node; generated artifacts remain tracked.

## 3. Final API decisions

### 3.1 Suite namespaces

`Tools` becomes a set of suite namespaces:

```ts
interface Tools {
  file: FileTools;
  search: SearchTools;
  process: ProcessTools;
  web: WebTools;
  browser: BrowserTools;
  lsp: LspTools;
  memory: MemoryTools;
}
```

Canonical paths:

```text
tools.file.read / write / edit / apply_edits / glob / grep
tools.search.code_search
tools.process.run / wait / kill
tools.web.fetch / search
tools.browser.open / close / list / read / click / type
tools.lsp.diagnostics / document_symbol / workspace_symbol / hover /
        find_references / go_to_definition / go_to_implementation
tools.memory.list / upsert / delete
```

Runtime tool IDs remain stable (`read`, `webfetch`, `browser_open`,
`code_search`, `memory.list`, etc.). The SDK binding path is separate from the
runtime tool name and must be mapped explicitly by the bridge.

### 3.2 Failure model

```ts
type ToolSuccess<T> = T & { ok: true };
type ToolResult<T> = ToolSuccess<T> | ToolFailure;

type ToolFailure = {
  ok: false;
  reason:
    | "not_found"
    | "no_match"
    | "invalid_args"
    | "empty"
    | "permission"
    | "io"
    | "conflict"
    | "stale"
    | "unavailable"
    | "still_alive";
  message: string;
};
```

Rules:

- success payload types do not declare `ok`; the generated output schema
  injects `ok: { const: true }` and the host injects it at the program boundary;
- a top-level tool failure is `ToolFailure` (resolved) or `ToolError` (thrown)
  according to the protocol/runtime class; domain failures resolve;
- batch item failures remain item-level types and do not create a second
  top-level failure model;
- `code_search` engine not-ready is `unavailable`, not a successful empty hit
  list;
- command `ok` no longer means command exit success; `exit_code` carries the
  process status. A still-alive kill resolves `ToolFailure` with reason
  `still_alive`; an exited/killed command remains successful command data.

The exact `ToolError` vs `ToolFailure` policy goes next to the types in
`prelude.d.ts`, not in a long prelude paragraph.

### 3.3 CodeSearch

```ts
type CodeSearchInput = {
  query: string;
  include_pattern?: string;
  min_score?: number;
  top_k?: Integer;
};

type CodeSearchData = {
  hits: CodeSearchHit[];
  message?: string;
};
```

- actual search + hits: success;
- actual search + zero hits: success with `hits: []` and a message;
- engine off/warming/busy/failed: `ToolFailure` reason `unavailable`;
- remove `CodeSearchStatus` and `CodeSearchData.ok:boolean`.

### 3.4 Memory

```ts
type NoArgs = {};

type MemoryEntry = {
  id: string;
  text: string;
  /** Milliseconds since the Unix epoch. */
  updated_at: Integer;
};

type MemoryListData = {
  count: Integer;
  entries: MemoryEntry[];
  limit: Integer;
  total: Integer;
  total_chars: Integer;
};

type MemoryUpsertEntry = {
  text: string;
  id?: string;
};

type MemoryUpsertData = {
  added: string[];
  count: Integer;
  rejected: RejectedItem[];
  total_chars: Integer;
  updated: string[];
};

type MemoryDeleteInput = { ids: string[] };

type MemoryDeleteData = {
  count: Integer;
  deleted: string[];
  rejected: RejectedItem[];
};

interface MemoryTools {
  /** Return the authoritative current session memory snapshot. */
  list(args?: NoArgs): Promise<ToolResult<MemoryListData>>;
  upsert(args: { entries: MemoryUpsertEntry[] }): Promise<ToolResult<MemoryUpsertData>>;
  delete(args: MemoryDeleteInput): Promise<ToolResult<MemoryDeleteData>>;
}
```

- delete `MemoryKind`, all `kind` fields, `kind` filters, and `invalid_kind`;
- delete the `memory.list` query filter: the bounded full snapshot is the
  authoritative view; JS can filter locally when needed;
- memory is session-scoped and survives compaction; the reminder re-injects the
  current entries after compaction;
- `memory.list` must be atomic: one call returns all current entries with ids
  and text.

### 3.5 No-argument tools

`browser_list` and `memory.list` use optional `NoArgs`:

```ts
browser_list(args?: NoArgs): Promise<ToolResult<BrowserListData>>;
memory.list(args?: NoArgs): Promise<ToolResult<MemoryListData>>;
```

The bridge maps zero JavaScript arguments to `{}` and validates against the
empty object input schema. Tools with required fields still resolve missing
arguments as `invalid_args`.

### 3.6 Prelude

Keep only concepts and runtime facts that types cannot express:

1. `run_code` purpose: token efficiency, fewer round trips, information
   density, orchestration/parallelism/aggregation; think before calling.
2. async function body, top-level `await`/`return`, return value is result.
3. sandbox boundary: tools handle side effects; `fs` is read-only; no direct
   network/process access.
4. call shape: `tools.<suite>.<member>(args)`, optional args for no-arg tools.
5. lifecycle/error class: resolved `ToolFailure` vs thrown `ToolError`,
   cancellation/budget/hard limits cannot be caught.
6. only rare non-derivable semantics such as non-finite output coercion.

Remove: workflow tutorials, per-tool examples, duplicate `libs`/`fs` docs,
duplicated error explanations, and generic "every resolved call" prose.

Move semantics next to types:

- `ToolError` / `ToolFailure` / `ToolResult` in `prelude.d.ts`;
- memory session/reinjection and snapshot semantics in `MemoryTools`;
- code-search empty-result semantics in `CodeSearchData`;
- field constraints and defaults in `api.d.ts` comments;
- `libs`/`fs` semantics in `libs.d.ts`.

## 4. Implementation plan

### Phase 0 - commit baseline

- Commit current JS-first refactor and generated artifacts.
- Result: `945dcbd4`.

### Phase 1 - API source reshape

- Restructure `sdk/src/api.d.ts` into suite interfaces and a `Tools` namespace.
- Rename SDK member paths while preserving `@tool` runtime ids.
- Remove payload-level `ok`; add `ToolSuccess<T>` / `ToolResult<T>` in prelude.
- Simplify CodeSearch types and remove `status`.
- Remove memory `kind`/query and add atomic `list(args?: NoArgs)`.
- Make `browser_list` and `memory.list` optional-argument tools.
- Update nearby field docs and casing where compatible.

### Phase 2 - generator / SDK assembler

- Generate output schemas with required `ok:true` injected.
- Render nested suite namespaces for all built-ins and custom suites.
- Keep deterministic suite/member ordering and type union deduplication.
- Support empty `NoArgs` input schemas and zero-argument bridge calls.
- Regenerate all committed artifacts; drift check must stay exact.

### Phase 3 - Rust bridge and runtime

- Add `CodeSurface.bindings`: SDK path -> runtime tool name.
- Install bridge functions by SDK path, resolve execution by runtime name.
- Update validation, ledger labels, and error messages to use SDK paths.
- Map zero-argument invocations to `{}`.
- Update CodeSearch failure paths to `unavailable` / `ToolFailure`.
- Remove memory kind and query behavior from session storage, reminder text,
  tool handlers, and tests.
- Preserve cross-compaction memory re-injection.

### Phase 4 - prelude and docs

- Replace `prelude.md` with the short philosophy/runtime contract.
- Move detailed semantics into `prelude.d.ts`, `libs.d.ts`, and `api.d.ts`.
- Remove workflow examples and duplicated explanations.
- Regenerate `sdk/generated/all-code-sdk.txt` and `.d.ts`.

### Phase 5 - tests and closure

- Update contract snapshot and named registry tests.
- Update code-surface/bridge tests for `tools.file.*`, `tools.process.*`, etc.
- Add tests for no-argument calls, CodeSearch empty vs unavailable, memory
  snapshot/reinjection without kind, and failure-envelope behavior.
- Run `cargo build`, `cargo check`, full `cargo test`, `pnpm --dir sdk check`.
- Keep CI SDK check wired for Linux and packaging/release workflows.

## 5. Acceptance criteria

- `tools.<suite>.<member>` is the only built-in call shape.
- No success payload has an `ok:boolean`; success `ok:true` comes from the
  envelope/schema.
- CodeSearch returns `ok:true` for a real search, including zero hits; engine
  readiness failures are `unavailable`, never fake empty success.
- `memory.list()` takes no argument and returns the complete current session
  memory snapshot; no `kind` or `query` exists in the public memory API.
- Memory survives compaction and is re-injected unchanged.
- `browser_list()` and `memory.list()` work without `{}`.
- No workflow examples remain in prelude.
- Full SDK is deterministic and byte-identical between Node generator and Rust
  assembler.
- Generator drift fails `pnpm --dir sdk check`; CI/release runs it.
- `cargo build` remains Node-free.

## 6. Do not

- Do not add compatibility shims or keep old names for safety.
- Do not reintroduce Rust-owned code-tool schemas or direct fallbacks.
- Do not keep `kind`, `status`, or payload-level `ok` as parallel state.
- Do not add examples or explanatory prose that merely repeats types.
- Do not let custom/MCP schema fallback leak into built-in declarations.

## 7. References

- `dev/plans/js-first-tool-contract-refactor.md`
- `dev/plans/js-first-tool-contract-result.md`
- `sdk/src/api.d.ts`
- `sdk/scripts/generate-contract.mjs`
- `src/tool/code_contract.rs`
- `src/tool/code_sdk.rs`
- `src/tools/code.rs`
- `src/tools/code_search.rs`
- `src/tools/memory.rs`
- `src/session/memory.rs`
- `src/session/manager.rs`
- `tests/code_surface.rs`
- `tests/run_code_model_view.rs`

## 8. Verification

- `cargo build` passes.
- `cargo check` passes.
- `cargo test --no-run` passes.
- Full `cargo test --no-fail-fast` with the four environment-sensitive tests
  filtered reports 76 targets and 1209 passed, 0 failed, 2 ignored.
  Filtered:
  - `subagent::spawn_contract`
  - `non_event_stream_content_type_surfaces_the_proxy_body`
  - `websocket_terminals_are_owner_isolated_and_disconnect_cleaned`
  - `code_search_lifecycle_rebuild_stale_consume_stop`
- `tsc -p sdk/tsconfig.json --noEmit` passes.
- `tsc -p sdk/tsconfig.generated.json --noEmit` passes.
- `node sdk/scripts/generate-contract.mjs --check` passes with no drift.
- The combined commands are what `pnpm --dir sdk check` runs in CI.
