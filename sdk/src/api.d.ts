/** Whole JSON number. The host rejects fractional values. @asType integer */
type Integer = number;

/** Workspace-relative path, or an absolute path when explicitly allowed. */
type Path = string;

/** Empty input for tools that take no arguments. */
type NoArgs = {};

type Location = {
  /** Source file path. */
  path: string;
  /** 1-based line number. */
  line: Integer;
  /** 1-based UTF-16 column on that line. */
  character: Integer;
  /** Optional end column. */
  end_character?: Integer;
  /** Optional end line. */
  end_line?: Integer;
};

type SymbolInfo = {
  /** Symbol name. */
  name: string;
  /** Symbol kind label supplied by the language server. */
  kind: string;
  /** 1-based line number. */
  line: Integer;
  /** 1-based UTF-16 column on that line. */
  character: Integer;
  /** Optional end column. */
  end_character?: Integer;
  /** Optional end line. */
  end_line?: Integer;
  /** Optional source file path. */
  path?: string;
};

type PathInput = {
  /** Source file under the workspace; workspace-relative is preferred. */
  path: Path;
};

type RangeInput = {
  /** 1-based line number. */
  line: Integer;
  /** 1-based UTF-16 column on that line. */
  character: Integer;
  /** Source file under the workspace; workspace-relative is preferred. */
  path: Path;
};

type ClampedParam = {
  /** Parameter name as the caller wrote it. */
  parameter: string;
  /** Value the caller asked for. */
  requested: number;
  /** Value the tool used instead. */
  applied: number;
};

/** Truthful index state of the language server that produced this result. */
type LspDataState = "ok" | "partial" | "unknown";

type SymbolsData = {
  symbols: SymbolInfo[];
  /** ok means the index is ready, partial that it is still building, and unknown that the server never reported it. An empty value under partial/unknown is not a confirmed miss. */
  state: LspDataState;
  /** True when the cap cut the list: more results exist. */
  truncated: boolean;
};

type LocationsData = {
  locations: Location[];
  /** ok means the index is ready, partial that it is still building, and unknown that the server never reported it. An empty value under partial/unknown is not a confirmed miss. */
  state: LspDataState;
  /** True when the cap cut the list: more results exist. */
  truncated: boolean;
};

/** One warning a tool attached to a successful write. */
type ToolDiagnostic = {
  /** Machine code such as `lsp_error`. */
  code: string;
  /** Severity name mapped from the LSP number. */
  severity: "info" | "warning";
  /** Human-readable summary. */
  message: string;
  /** Extra structured detail, such as the LSP error block under `text`. */
  details?: unknown;
};

type PathRisk = {
  /** The target resolves outside the workspace root. */
  outside_workspace: boolean;
  /** The target is a sensitive system location. */
  sensitive: boolean;
  /** Absolute path the target resolved to; present when it is outside. */
  resolved?: string;
};

type BrowserPage = {
  active: boolean;
  agentOwned: boolean;
  id: string;
  title: string;
  url: string;
  visible: boolean;
};

type EditInput = {
  /** Replacement text. Use an empty string to delete old_string. */
  new_string: string;
  /** Exact file text to find. Must match exactly one place. */
  old_string: string;
  /** Prefer a workspace-relative path. */
  path: Path;
};

type RejectedItem = {
  /** 0-based input item index. */
  index: Integer;
  /** empty_text | text_too_long | invalid_id | unknown_id | limit_exceeded. */
  reason: string;
  id?: string;
  /** Which limit the reason names, and by how much this item misses it. */
  detail?: string;
};



interface FileTools {
  /** Read lines from one file or archived session transcript. @tool read @suite file */
  read(args: {
    path: Path; // Prefer a workspace-relative path. Past session transcripts are readable at .litecode/sessions/<session_id>.md.
    start_line?: Integer; // First line to read, 1-based, inclusive. @default 1
    limit?: Integer; // Lines to return. 1–5000. @default 2000
  }): Promise<ToolResult<{
    lines: string[]; // Line contents for this page, in file order.
    path: string;
    start_line: Integer;
    total_lines: Integer; // Lines in the file; `has_more` compares against it.
    has_more: boolean; // True when this page stops before the end of the file.
    skipped_in_context: Integer; // Lines the window covered but left out because they are already in the model's context. A gap in `lines` is explained by this number.
    lines_capped?: Integer[]; // Lines shortened to the per-line character cap, by number.
    clamped?: ClampedParam[]; // Parameters adjusted to a legal value; absent when nothing moved.
  }>>;
  /** Replace a file's contents; optional hash and create-only guards. @tool write @suite file */
  write(args: {
    content: string; // Content to write to the file.
    path: Path; // Prefer a workspace-relative path.
    create_only?: boolean; // If true, refuse to overwrite an existing file. @default false
    expected_hash?: string; // sha256 of the current file bytes. Omit to skip the check. A mismatch is ToolFailure reason stale.
  }): Promise<ToolResult<{
    created: boolean; // True when the file did not exist before.
    // File identity after the write.
    file: {
      bytes: Integer; // File size in bytes.
      hash: string; // sha256 hex of the file bytes.
      path: string; // Workspace-relative path inside the workspace; requested path outside it.
      abs?: string; // Absolute path when the file is outside the workspace.
    };
    path_risk?: PathRisk; // Outside-workspace or sensitive-path target; absent when there is none.
    diagnostics?: ToolDiagnostic[]; // Warnings recorded after the write; absent when the write was clean.
  }>>;
  /** Apply one exact-string replacement. @tool edit @suite file */
  edit(args: EditInput): Promise<ToolResult<
    // Successful single-edit payload. Failures use ToolFailure.
    {
      path: string;
      path_risk?: PathRisk; // Outside-workspace or sensitive-path target; absent when there is none.
      diagnostics?: ToolDiagnostic[]; // Warnings recorded after the edit; absent when the edit was clean.
    }
  >>;
  /** Apply several edit replacements serially. @tool apply_edits @suite file */
  apply_edits(args: {
    edits: EditInput[]; // Replacements applied serially; later items on the same path see earlier successful writes.
  }): Promise<ToolResult<{
    // Same length and order as edits.
    results: Array<
      // One item in a serial apply_edits batch. Item failures stay item-level.
      {
        ok: boolean;
        path: string;
        reason: ("not_found" | "no_match" | "ambiguous" | "invalid_args" | "empty" | "permission" | "io") | null; // Why this item failed; `ambiguous` means `old_string` matched more than once. `null` when `ok` is true.
        detail: string; // One plain sentence: why the item failed, with line hints; empty when `ok` is true.
        nearest_line: Integer | null; // 1-based line of the closest match when `old_string` was not found; `null` otherwise.
        path_risk?: PathRisk; // Outside-workspace or sensitive-path target; absent when there is none.
        diagnostics?: ToolDiagnostic[]; // Warnings recorded after the edit; absent when there were none.
      }
    >;
  }>>;
  /** Find files by path glob. @tool glob @suite file */
  glob(args: {
    pattern: string; // Glob relative to path, or to the workspace root when path is omitted.
    limit?: Integer; // Maximum paths to return. 1–5000. @default 1000
    path?: Path; // Optional directory to search under.
  }): Promise<ToolResult<{
    paths: string[]; // Matching paths, sorted by depth, then parent directory, then name.
    total: Integer; // Matches found before the limit was applied.
    truncated: boolean; // True when paths holds only the first limit matches.
    outside_workspace: boolean; // True when some returned paths are outside the workspace (absolute).
    empty_reason?: "hidden_by_filters" | "no_match"; // Why `paths` is empty; absent when it is not. Only what glob can produce: the searched path is excluded by default filters, or nothing matched.
    clamped?: ClampedParam[]; // Parameters adjusted to a legal value; absent when nothing moved.
  }>>;
  /** Search file contents. @tool grep @suite file */
  grep(args: {
    pattern: string; // Regular expression over file contents, unless literal is true.
    before?: Integer; // Lines of context before each hit. 0–10. @default 0
    after?: Integer; // Lines of context after each hit. 0–10. @default 0
    glob?: string; // One glob or comma-separated globs. A leading ! excludes. Relative to path.
    ignore_case?: boolean; // Ignore case when matching. @default false
    limit?: Integer; // Maximum hits to return. 1–100. @default 40
    literal?: boolean; // Search for pattern as plain text. @default false
    path?: Path; // File or directory to search. Defaults to the workspace.
  }): Promise<ToolResult<{
    hits: Array<
      // One match. Context, when requested, is folded into `text` as one block.
      {
        path: string; // Source file path.
        line: Integer; // 1-based match line.
        text: string; // Matching line text, capped at 500 characters. With `before`/`after` this is a whole block in ripgrep notation, newline-separated: `12- before`, `13: match`, `14+ after` (real file line numbers).
      }
    >;
    truncated?: boolean; // True when a limit or the token budget cut the result short.
    empty_reason?: "no_files" | "glob_no_files" | "no_match" | "file_unreadable" | "binary" | "hidden_by_filters"; // Why `hits` is empty; absent when it is not. Only what grep can produce: nothing to search, the glob filter matched no file, no match, the file could not be read as text, the file is binary, or the searched path is excluded by default filters.
    total_hits?: Integer; // Matches found before the result was cut; absent when the scan stopped at a limit and never counted the rest. Do not treat a missing value as zero.
    at_limit_cap: boolean; // True when the hit limit stopped the scan.
    lines_truncated: boolean; // True when a match or context line was shortened to the per-line cap.
    context_omitted: boolean; // True when context lines were dropped to fit the token budget.
    output_capped: boolean; // True when hits were dropped to fit the token budget.
    clamped?: ClampedParam[]; // Parameters adjusted to a legal value; absent when nothing moved.
  }>>;
}

interface SearchTools {
  /** Semantic search over the workspace code index. @tool code_search @suite search */
  code_search(args: {
    query: string; // One natural-language or code-token query.
    include_pattern?: string; // Optional glob filter for workspace paths.
    min_score?: number; // Minimum raw dense cosine similarity for dense-only hits. 0–1. Lexical hits are not subject to this floor; 0 disables it. @default 0.8
    top_k?: Integer; // Maximum hits. 1–20. @default 8
  }): Promise<ToolResult<{
    // Empty if the search completed but found no matching chunks.
    hits: {
      path: string; // Source file path.
      anchor_line: Integer; // Query-relevant anchor line.
      start_line: Integer; // First line in the returned snippet.
      end_line: Integer; // Last line in the returned snippet.
      symbol?: string; // Optional AST symbol enclosing the anchor.
      via: "lex" | "dense" | "both"; // Which retrieval leg(s) admitted this chunk.
      dense_similarity: number; // Raw dense cosine similarity; 0.0 means the hit is lexical-only.
      text: string; // The anchor line itself, cheap enough for a table cell.
      snippet: string; // Source excerpt, at most 24 lines; overlong lines are capped.
    }[];
    clamped?: ClampedParam[]; // Parameters adjusted to a legal value; absent when nothing moved.
  }>>;
  /** Search past session transcripts. @tool session_search @suite search */
  session_search(args: {
    query: string; // What to find in past session transcripts. Literal words or a phrase.
    session_id?: string; // Optional session scope: full id, unique prefix, or the handle from a result.
    own_session?: boolean; // Search only this session's transcript: the turns already folded out of the live context window. Cannot be combined with session_id.
    limit?: Integer; // Maximum rows. 1–50. @default 20
  }): Promise<ToolResult<{
    // Flat rank order; not grouped by session.
    hits: {
      session: string; // Shortest unique session handle, also accepted by `session_id`.
      bot_name?: string; // Derived bot name for this session, if any. Never a stored session field.
      age: string; // Relative age of the session's last update.
      line: Integer; // 1-based rendered transcript line of the hit.
      role: "conversation" | "reasoning" | "action" | "outcome" | "unknown"; // Content role of the row.
      via: string; // Evidence that found this row, in rank order: `exact`, `prox`, `lex`, `sem`, `fuzzy`, joined by `+`.
      summary: string; // Collapsed one-line hit text.
    }[];
    total: Integer; // Total matching rows before `limit` is applied.
    truncated: boolean; // True when `total` exceeds the returned rows.
    semantic: boolean; // False when the semantic leg was unavailable: the hits are lexical only.
    clamped?: ClampedParam[]; // Parameters adjusted to a legal value; absent when nothing moved.
  }>>;
}

type ExecCommandData = {
  /** Live session id; present only while the process is still running. */
  session_id?: string;
  /** Process exit code when it finished before this call returned. */
  exit_code?: Integer | null;
  /** Time spent inside this exec_command / write_stdin call. */
  wall_time_seconds: number;
  /** Incremental output since the previous exec call on this session. */
  output: string;
  /** Approximate original output size before the call's token budget. */
  original_token_count?: Integer;
  /** Output was clipped by max_output_tokens. */
  truncated?: boolean;
};

interface ProcessTools {
  /** Start a command in a PTY, returning output or a session id for ongoing interaction. @tool exec_command @suite process */
  exec_command(args: {
    cmd: string; // Shell command to execute.
    workdir?: string; // Working directory; defaults to the turn cwd and must stay inside it.
    tty?: boolean; // True allocates a PTY; false or omitted uses plain pipes.
    yield_time_ms?: Integer; // Wait before yielding output. 250-30000 ms; default 10000.
    max_output_tokens?: Integer; // Output token budget for this call; default 10000.
  }): Promise<ToolResult<ExecCommandData>>;
  /** Write characters to an existing exec session and return recent output. @tool write_stdin @suite process */
  write_stdin(args: {
    session_id: string; // Session id from exec_command.
    chars?: string; // Bytes to write; empty polls. "\u0003" sends SIGINT to the process group.
    yield_time_ms?: Integer; // Wait before yielding output. Empty polls default 5000 ms; writes default 250 ms.
    max_output_tokens?: Integer; // Output token budget for this call; default 10000.
  }): Promise<ToolResult<ExecCommandData>>;
}

interface WebTools {
  /** Fetch one http(s) URL. @tool webfetch @suite web */
  fetch(args: {
    url: string; // URL to fetch. Only http(s) URLs are accepted.
    format?: "text" | "markdown" | "html"; // Desired output format. @default "markdown"
  }): Promise<ToolResult<{
    body?: string; // Empty for a non-2xx answer: the status carries the fact.
    format?: "text" | "markdown" | "html";
    status?: Integer; // HTTP status of the final response, 2xx or not.
    url?: string;
  }>>;
  /** Search the web. @tool websearch @suite web */
  search(args: {
    query: string; // Search query.
    top_k?: Integer; // Number of results, best first. 1–100. @default 10
  }): Promise<ToolResult<{
    // At most top_k hits, best first; empty when the search found nothing.
    results: {
      snippet: string; // Query-relevant excerpt from the page, as returned by the provider.
      title: string;
      url: string;
    }[];
    query?: string;
    clamped?: ClampedParam[]; // Parameters adjusted to a legal value; absent when nothing moved.
  }>>;
}

interface BrowserTools {
  /** Open a visible browser tab. @tool browser_open @suite browser */
  open(args: {
    url: string; // http(s) URL to open in a visible browser tab.
  }): Promise<ToolResult<BrowserPage>>;
  /** Close an agent-owned browser tab. @tool browser_close @suite browser */
  close(args: {
    page_id: string; // Browser page id from browser_open or browser_list.
  }): Promise<ToolResult<{
    closed: boolean;
    id: string;
    fallback?: boolean;
  }>>;
  /** List open browser tabs. @tool browser_list @suite browser */
  list(args?: NoArgs): Promise<ToolResult<{
    active_id: string;
    pages: BrowserPage[];
  }>>;
  /** Read the current visible browser page. @tool browser_read @suite browser */
  read(args: {
    max_chars?: Integer; // Maximum body text characters. 1000–30000. @default 12000
    page_id?: string; // Optional page id. Omit for the active visible tab.
  }): Promise<ToolResult<{
    elements: {
      ref: string; // Element reference from the latest browser_read.
      tag: string;
      text: string;
      href?: string;
      type?: string;
    }[];
    message?: string; // Set when max_chars was clamped into range.
    page_id: string;
    text: string;
    title: string;
    truncated: boolean;
    url: string;
  }>>;
  /** Click one interactive element on the visible page. @tool browser_click @suite browser */
  click(args: {
    ref: string; // Element reference from the latest browser_read.
    page_id?: string; // Optional page id. Omit for the active visible tab.
  }): Promise<ToolResult<{
    page_id: string;
    ref: string;
    tag: string;
    text: string;
    title: string;
    url: string;
  }>>;
  /** Type text into one input on the visible page. @tool browser_type @suite browser */
  type(args: {
    ref: string; // Input reference from the latest browser_read.
    text: string; // Text to set on the focused input.
    page_id?: string; // Optional page id. Omit for the active visible tab.
  }): Promise<ToolResult<{
    page_id: string;
    ref: string;
    tag: string;
    title: string;
    url: string;
  }>>;
}

interface LspTools {
  /** List diagnostics for one source file. @tool diagnostics @suite lsp */
  diagnostics(args: PathInput): Promise<ToolResult<{
    items: {
      severity: string; // Severity name mapped from the LSP number: Error, Warning, Information, Hint; Unknown for anything else.
      path: string; // Source file path.
      line: Integer; // 1-based line number.
      character: Integer; // 1-based UTF-16 column on that line.
      message: string; // Human-readable diagnostic.
    }[];
    state: LspDataState; // ok means the index is ready, partial that it is still building, and unknown that the server never reported it. An empty value under partial/unknown is not a confirmed miss.
    truncated: boolean; // True when the cap cut the list: more results exist.
  }>>;
  /** List symbols in one source file. @tool document_symbol @suite lsp */
  document_symbol(args: PathInput): Promise<ToolResult<SymbolsData>>;
  /** Search symbols across the workspace. @tool workspace_symbol @suite lsp */
  workspace_symbol(args: {
    query: string; // Symbol name or fragment to search across the workspace.
    path?: string; // Optional source file that selects/starts the project's language server. Required only when more than one server is running.
  }): Promise<ToolResult<SymbolsData>>;
  /** Fetch hover docs or type information at a file position. @tool hover @suite lsp */
  hover(args: RangeInput): Promise<ToolResult<
    // One hover answer. A hover is a single value, so there is no truncation.
    {
      markdown: string; // Flattened hover text, not raw LSP JSON.
      state: LspDataState; // ok means the index is ready, partial that it is still building, and unknown that the server never reported it. An empty value under partial/unknown is not a confirmed miss.
    }
  >>;
  /** List references at a file position. @tool find_references @suite lsp */
  find_references(args: RangeInput): Promise<ToolResult<LocationsData>>;
  /** Jump to definition at a file position. @tool go_to_definition @suite lsp */
  go_to_definition(args: RangeInput): Promise<ToolResult<LocationsData>>;
  /** Jump to implementations at a file position. @tool go_to_implementation @suite lsp */
  go_to_implementation(args: RangeInput): Promise<ToolResult<LocationsData>>;
}

interface MemoryTools {
  /** Return the authoritative current session memory snapshot; memory survives compaction and is re-injected. @tool memory.list @suite memory */
  list(args?: NoArgs): Promise<ToolResult<{
    count: Integer; // Number of entries in this snapshot.
    // Authoritative current entries, in insertion order.
    entries: {
      id: string;
      text: string;
      updated_at: Integer; // Milliseconds since Unix epoch.
    }[];
    limit: Integer; // Maximum entries the session can hold.
    total: Integer; // Entries in the whole session memory.
    total_chars: Integer; // Characters across all entry texts.
  }>>;
  /** Add or update small, durable session memory entries. @tool memory.upsert @suite memory */
  upsert(args: {
    // Memory entries to add or update.
    entries: {
      text: string; // Entry text, trimmed. 1-200 characters; duplicate text updates the existing entry.
      id?: string; // Optional existing id. Omit to add or update by text.
    }[];
  }): Promise<ToolResult<{
    added: string[]; // New ids.
    count: Integer; // Entries after the batch.
    rejected: RejectedItem[];
    total_chars: Integer; // Characters after the batch.
    updated: string[]; // Updated ids.
  }>>;
  /** Delete entries by id from the current session's working memory. @tool memory.delete @suite memory */
  delete(args: {
    ids: string[]; // Entry ids to delete.
  }): Promise<ToolResult<{
    count: Integer; // Entries after the batch.
    deleted: string[]; // Deleted ids.
    rejected: RejectedItem[];
  }>>;
}

interface Tools {
  file: FileTools;
  search: SearchTools;
  process: ProcessTools;
  web: WebTools;
  browser: BrowserTools;
  lsp: LspTools;
  memory: MemoryTools;
}

declare const tools: Tools;
