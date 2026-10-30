declare class ToolError extends Error {
  readonly name: "ToolError";
  readonly code: string;
  readonly details?: unknown;
  readonly retryable?: true;
  readonly tool: string;
  readonly seq: number;
}

/**
 * A resolved domain failure. ToolError is thrown for protocol and hard
 * execution failures; this shape is the normal failure branch of ToolResult.
 */
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

/** A successful tool result. The host adds this envelope; payloads omit ok. */
type ToolSuccess<T> = T & { ok: true };

type ToolResult<T> = ToolSuccess<T> | ToolFailure;

type CsvOptions = {
  /** Field separator; one character. Default ",". */
  delimiter?: string;
  /** Quote character; one character. Default is a double quote. */
  quote?: string;
  /** Trim fields while parsing. Default false. */
  trim?: boolean;
};

type HistogramBin = {
  start: number;
  end: number;
  count: number;
};

type DiffLine = {
  kind: "equal" | "insert" | "delete";
  text: string;
};

declare const libs: {
  stats: {
    sum(values: number[]): number;
    mean(values: number[]): number;
    median(values: number[]): number;
    variance(values: number[], sample?: boolean): number;
    stddev(values: number[], sample?: boolean): number;
    quantile(values: number[], q: number): number;
    histogram(values: number[], bins?: number): HistogramBin[];
    min(values: number[]): number;
    max(values: number[]): number;
  };
  csv: {
    parse(text: string, options?: CsvOptions): string[][];
    format(rows: Array<Array<unknown>>, options?: CsvOptions): string;
  };
  text: {
    lines(text: string): string[];
    words(text: string): string[];
    count(text: string, needle: string): number;
    normalizeNewlines(text: string): string;
    truncate(text: string, maxChars: number, suffix?: string): string;
    diffLines(left: string, right: string): DiffLine[];
  };
  data: {
    groupBy<T>(rows: T[], key: string | ((row: T) => unknown)): Record<string, T[]>;
    countBy<T>(rows: T[], key: string | ((row: T) => unknown)): Record<string, number>;
    unique<T>(values: T[]): T[];
    sortBy<T>(rows: T[], key: string | ((row: T) => unknown), direction?: "asc" | "desc"): T[];
  };
};

type FsFailure = { ok: false; reason: "not_found" | "permission" | "io"; message: string };

/** Read-only workspace probes. Synchronous; paths are workspace-relative. */
declare const fs: {
  /** One level, sorted by name, at most 500 entries (message says when cut). */
  ls(path: string): { ok: true; entries: Array<{ name: string; type: "file" | "dir" | "symlink" | "other"; size: number }>; message?: string } | FsFailure;
  /** mtime is ms since the Unix epoch. */
  stat(path: string): { ok: true; type: "file" | "dir" | "symlink" | "other"; size: number; mtime: number } | FsFailure;
  exists(path: string): boolean;
};

type ApplyEditsData = {
  /** Same length and order as edits. */
  results: EditResult[];
};

type ApplyEditsInput = {
  /** Replacements applied serially; later items on the same path see earlier successful writes. */
  edits: EditInput[];
};

type BrowserClickData = {
  page_id: string;
  ref: string;
  tag: string;
  text: string;
  title: string;
  url: string;
};

type BrowserClickInput = {
  /** Element reference from the latest browser_read. */
  ref: string;
  /** Optional page id. Omit for the active visible tab. */
  page_id?: string;
};

type BrowserCloseData = {
  closed: boolean;
  id: string;
  fallback?: boolean;
};

type BrowserCloseInput = {
  /** Browser page id from browser_open or browser_list. */
  page_id: string;
};

type BrowserElement = {
  /** Element reference from the latest browser_read. */
  ref: string;
  tag: string;
  text: string;
  href?: string;
  type?: string;
};

type BrowserListData = {
  active_id: string;
  pages: BrowserPage[];
};

type BrowserOpenData = BrowserPage;

type BrowserOpenInput = {
  /** http(s) URL to open in a visible browser tab. */
  url: string;
};

type BrowserPage = {
  active: boolean;
  agentOwned: boolean;
  id: string;
  title: string;
  url: string;
  visible: boolean;
};

type BrowserReadData = {
  elements: BrowserElement[];
  page_id: string;
  text: string;
  title: string;
  truncated: boolean;
  url: string;
};

type BrowserReadInput = {
  /** Maximum body text characters. @default 12000 @minimum 1000 @maximum 30000 */
  max_chars?: Integer;
  /** Optional page id. Omit for the active visible tab. */
  page_id?: string;
};

type BrowserTypeData = {
  page_id: string;
  ref: string;
  tag: string;
  title: string;
  url: string;
};

type BrowserTypeInput = {
  /** Input reference from the latest browser_read. */
  ref: string;
  /** Text to set on the focused input. */
  text: string;
  /** Optional page id. Omit for the active visible tab. */
  page_id?: string;
};

type CodeSearchData = {
  /** Empty if the search completed but found no matching chunks. */
  hits: CodeSearchHit[];
  message?: string;
};

type CodeSearchHit = {
  /** Source file path. */
  path: string;
  /** Query-relevant anchor line. */
  anchor_line: Integer;
  /** First line in the returned snippet. */
  start_line: Integer;
  /** Last line in the returned snippet. */
  end_line: Integer;
  /** Optional AST symbol enclosing the anchor. */
  symbol?: string;
  /** Source excerpt, at most 24 lines; overlong lines are capped. */
  snippet: string;
};

type CodeSearchInput = {
  /** One natural-language or code-token query. */
  query: string;
  /** Optional glob filter for workspace paths. */
  include_pattern?: string;
  /** Minimum raw dense cosine similarity. @default 0.8 @minimum 0 @maximum 1 */
  min_score?: number;
  /** Maximum hits. @default 8 @minimum 1 @maximum 20 */
  top_k?: Integer;
};

type CommandData = {
  /** Process exit code when finished; null when still running or unavailable. */
  exit_code: Integer | null;
  /** While exit_code is null, use with tools.process.wait / tools.process.kill in this session. */
  id: string;
  /** Stdout plus truncation notes, spill path, or status line. Always present, may be empty. */
  output: string;
  /** Shell that ran the command. */
  shell: string;
  /** Stderr plus truncation notes. Empty when nothing was written. */
  stderr: string;
  /** True when wait observes a kill. */
  killed?: boolean;
  /** Warning about the command itself or a still-alive signal. */
  message?: string;
};

type DiagnosticItem = {
  /** Severity label supplied by the language server. */
  severity: string;
  /** Source file path. */
  path: string;
  /** 1-based line number. */
  line: Integer;
  /** 1-based UTF-16 column on that line. */
  character: Integer;
  /** Human-readable diagnostic. */
  message: string;
};

type DiagnosticsData = {
  items: DiagnosticItem[];
  message: string;
};

type DiagnosticsInput = PathInput;

type DocumentSymbolData = SymbolsData;

type DocumentSymbolInput = PathInput;

/** Successful single-edit payload. Failures use ToolFailure. */
type EditData = {
  message: string;
  path: string;
};

type EditFailureReason =
  | "not_found"
  | "no_match"
  | "invalid_args"
  | "empty"
  | "permission"
  | "io";

type EditInput = {
  /** Replacement text. Use an empty string to delete old_string. */
  new_string: string;
  /** Exact file text to find. Must match exactly one place. */
  old_string: string;
  /** Prefer a workspace-relative path. */
  path: Path;
};

/** One item in a serial apply_edits batch. Item failures stay item-level. */
type EditResult = {
  message: string;
  ok: boolean;
  path: string;
  reason?: EditFailureReason;
};

type FileRef = {
  /** File size in bytes. */
  bytes: Integer;
  /** sha256 hex of the file bytes. */
  hash: string;
  /** Workspace-relative path inside the workspace; requested path outside it. */
  path: string;
  /** Absolute path when the file is outside the workspace. */
  abs?: string;
};

type FindReferencesData = LocationsData;

type FindReferencesInput = RangeInput;

type GlobData = {
  message: string;
  /** Matching paths sorted by depth, then parent directory, then name. */
  paths: string[];
};

type GlobInput = {
  /** Glob relative to path, or to the workspace root when path is omitted. */
  pattern: string;
  /** Maximum paths to return. @default 1000 @minimum 1 @maximum 5000 */
  limit?: Integer;
  /** Optional directory to search under. */
  path?: Path;
};

type GoToDefinitionData = LocationsData;

type GoToDefinitionInput = RangeInput;

type GoToImplementationData = LocationsData;

type GoToImplementationInput = RangeInput;

type GrepData = {
  hits: GrepHit[];
  message: string;
  /** True when a limit or the 8K-token data guard removed results or context. */
  truncated?: boolean;
};

/** One match. Context, when requested, is folded into `text` as one block. */
type GrepHit = {
  /** Source file path. */
  path: string;
  /** 1-based match line. */
  line: Integer;
  /**
   * Matching line text, capped at 500 characters. With `before`/`after` this
   * is a whole block in ripgrep notation, newline-separated:
   * `12- before`, `13: match`, `14+ after` (real file line numbers).
   */
  text: string;
};

type GrepInput = {
  /** Regular expression over file contents, unless literal is true. */
  pattern: string;
  /** Lines of context before each hit. @default 0 @minimum 0 @maximum 5 */
  before?: Integer;
  /** Lines of context after each hit. @default 0 @minimum 0 @maximum 5 */
  after?: Integer;
  /** One glob or comma-separated globs. A leading ! excludes. Relative to path. */
  glob?: string;
  /** Ignore case when matching. @default false */
  ignore_case?: boolean;
  /** Maximum hits to return. @default 40 @minimum 1 @maximum 100 */
  limit?: Integer;
  /** Search for pattern as plain text. @default false */
  literal?: boolean;
  /** File or directory to search. Defaults to the workspace. */
  path?: Path;
};

type HoverData = {
  /** Flattened hover text, not raw LSP JSON. */
  markdown: string;
  message: string;
};

type HoverInput = RangeInput;

/** Whole JSON number. The host rejects fractional values. @asType integer */
type Integer = number;

type KillData = CommandData;

type KillInput = {
  /** Command.id from a tools.process.run or tools.process.wait result whose exit_code is null. */
  id: string;
};

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

type LocationsData = {
  locations: Location[];
  message: string;
};

type MemoryDeleteData = {
  /** Entries after the batch. */
  count: Integer;
  /** Deleted ids. */
  deleted: string[];
  rejected: RejectedItem[];
};

type MemoryDeleteInput = {
  /** Entry ids to delete. */
  ids: string[];
};

type MemoryEntry = {
  id: string;
  text: string;
  /** Milliseconds since Unix epoch. */
  updated_at: Integer;
};

type MemoryListData = {
  /** Number of entries in this snapshot. */
  count: Integer;
  /** Authoritative current entries, in insertion order. */
  entries: MemoryEntry[];
  /** Maximum entries the session can hold. */
  limit: Integer;
  /** Entries in the whole session memory. */
  total: Integer;
  /** Characters across all entry texts. */
  total_chars: Integer;
};

type MemoryUpsertData = {
  /** New ids. */
  added: string[];
  /** Entries after the batch. */
  count: Integer;
  rejected: RejectedItem[];
  /** Characters after the batch. */
  total_chars: Integer;
  /** Updated ids. */
  updated: string[];
};

type MemoryUpsertEntry = {
  /** Entry text, trimmed. 1-200 characters; duplicate text updates the existing entry. */
  text: string;
  /** Optional existing id. Omit to add or update by text. */
  id?: string;
};

type MemoryUpsertInput = {
  /** Memory entries to add or update. */
  entries: MemoryUpsertEntry[];
};

/** Empty input for tools that take no arguments. */
type NoArgs = {};

/** Workspace-relative path, or an absolute path when explicitly allowed. */
type Path = string;

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

type ReadData = {
  /** Line contents for this page, in file order. */
  lines: string[];
  message: string;
  path: string;
  start_line: Integer;
};

type ReadInput = {
  /** Prefer a workspace-relative path. Past session transcripts are readable at .litecode/sessions/<session_id>.md. */
  path: Path;
  /** First line to read, 1-based, inclusive. @default 1 @minimum 1 */
  start_line?: Integer;
  /** Maximum lines to return. @default 2000 @minimum 1 @maximum 5000 */
  limit?: Integer;
};

type RejectedItem = {
  /** 0-based input item index. */
  index: Integer;
  /** empty_text | text_too_long | invalid_id | unknown_id | limit_exceeded. */
  reason: string;
  id?: string;
};

/** Command payload shared by run, wait, and kill; exit_code carries command status. */
type RunData = CommandData;

type RunInput = {
  /** Shell command to run once in the workspace root. Chain with && for cd or env changes. */
  command: string;
  /**
   * Return immediately after spawn with exit_code:null and keep the job as a
   * wait/kill target. Default false: wait up to 30 seconds; longer jobs detach
   * automatically. @default false
   */
  run_in_background?: boolean;
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

type SymbolsData = {
  message: string;
  symbols: SymbolInfo[];
};

type WaitData = CommandData;

type WaitInput = {
  /** Command.id from a tools.process.run result whose exit_code is null. */
  id: string;
  /** Seconds to block waiting for exit. Values above 300 are clamped to 300. @minimum 1 */
  sec: Integer;
};

type WebfetchData = {
  message: string;
  body?: string;
  format?: WebfetchFormat;
  status?: Integer;
  url?: string;
};

type WebfetchFormat = "text" | "markdown" | "html";

type WebfetchInput = {
  /** URL to fetch. Only http(s) URLs are accepted. */
  url: string;
  /** Desired output format. @default "markdown" */
  format?: WebfetchFormat;
};

type WebsearchData = {
  message: string;
  /** At most top_k hits, best first. */
  results: WebsearchHit[];
  query?: string;
};

type WebsearchHit = {
  /** Leading page text, at most about 300 characters. */
  snippet: string;
  title: string;
  url: string;
};

type WebsearchInput = {
  /** Search query. */
  query: string;
  /** Number of results. Values above 10 are clamped to 10. @default 5 @minimum 1 */
  top_k?: Integer;
};

type WorkspaceSymbolData = SymbolsData;

type WorkspaceSymbolInput = {
  /** Symbol name or fragment to search across the workspace. */
  query: string;
};

type WriteData = {
  /** True when the file did not exist before. */
  created: boolean;
  /** File identity after the write. */
  file: FileRef;
  /** Post-write warnings such as LSP errors or path-risk notes. */
  message?: string;
};

type WriteInput = {
  /** Content to write to the file. */
  content: string;
  /** Prefer a workspace-relative path. */
  path: Path;
  /** If true, refuse to overwrite an existing file. @default false */
  create_only?: boolean;
  /** sha256 of the current file bytes. Omit to skip the check. A mismatch is ToolFailure reason stale. */
  expected_hash?: string;
};

interface Tools {
  // file
  file: {
    /**
     * Apply several edit replacements serially.
     */
    apply_edits(args: ApplyEditsInput): Promise<ToolResult<ApplyEditsData>>;
    /**
     * Apply one exact-string replacement.
     */
    edit(args: EditInput): Promise<ToolResult<EditData>>;
    /**
     * Find files by path glob.
     */
    glob(args: GlobInput): Promise<ToolResult<GlobData>>;
    /**
     * Search file contents.
     */
    grep(args: GrepInput): Promise<ToolResult<GrepData>>;
    /**
     * Read lines from one file or archived session transcript.
     */
    read(args: ReadInput): Promise<ToolResult<ReadData>>;
    /**
     * Replace a file's contents; optional hash and create-only guards.
     */
    write(args: WriteInput): Promise<ToolResult<WriteData>>;
  };

  // search
  search: {
    /**
     * Semantic search over the workspace code index.
     */
    code_search(args: CodeSearchInput): Promise<ToolResult<CodeSearchData>>;
  };

  // process
  process: {
    /**
     * Terminate a command job in this session.
     */
    kill(args: KillInput): Promise<ToolResult<KillData>>;
    /**
     * Start one shell command; use run_in_background for a non-blocking job.
     */
    run(args: RunInput): Promise<ToolResult<RunData>>;
    /**
     * Wait for a still-running command job; exit_code is the command status.
     */
    wait(args: WaitInput): Promise<ToolResult<WaitData>>;
  };

  // web
  web: {
    /**
     * Fetch one http(s) URL.
     */
    fetch(args: WebfetchInput): Promise<ToolResult<WebfetchData>>;
    /**
     * Search the web.
     */
    search(args: WebsearchInput): Promise<ToolResult<WebsearchData>>;
  };

  // browser
  browser: {
    /**
     * Click one interactive element on the visible page.
     */
    click(args: BrowserClickInput): Promise<ToolResult<BrowserClickData>>;
    /**
     * Close an agent-owned browser tab.
     */
    close(args: BrowserCloseInput): Promise<ToolResult<BrowserCloseData>>;
    /**
     * List open browser tabs.
     */
    list(args?: NoArgs): Promise<ToolResult<BrowserListData>>;
    /**
     * Open a visible browser tab.
     */
    open(args: BrowserOpenInput): Promise<ToolResult<BrowserOpenData>>;
    /**
     * Read the current visible browser page.
     */
    read(args: BrowserReadInput): Promise<ToolResult<BrowserReadData>>;
    /**
     * Type text into one input on the visible page.
     */
    type(args: BrowserTypeInput): Promise<ToolResult<BrowserTypeData>>;
  };

  // lsp
  lsp: {
    /**
     * List diagnostics for one source file.
     */
    diagnostics(args: DiagnosticsInput): Promise<ToolResult<DiagnosticsData>>;
    /**
     * List symbols in one source file.
     */
    document_symbol(args: DocumentSymbolInput): Promise<ToolResult<DocumentSymbolData>>;
    /**
     * List references at a file position.
     */
    find_references(args: FindReferencesInput): Promise<ToolResult<FindReferencesData>>;
    /**
     * Jump to definition at a file position.
     */
    go_to_definition(args: GoToDefinitionInput): Promise<ToolResult<GoToDefinitionData>>;
    /**
     * Jump to implementations at a file position.
     */
    go_to_implementation(args: GoToImplementationInput): Promise<ToolResult<GoToImplementationData>>;
    /**
     * Fetch hover docs or type information at a file position.
     */
    hover(args: HoverInput): Promise<ToolResult<HoverData>>;
    /**
     * Search symbols across the workspace.
     */
    workspace_symbol(args: WorkspaceSymbolInput): Promise<ToolResult<WorkspaceSymbolData>>;
  };

  // memory
  memory: {
    /**
     * Delete entries by id from the current session's working memory.
     */
    delete(args: MemoryDeleteInput): Promise<ToolResult<MemoryDeleteData>>;
    /**
     * Return the authoritative current session memory snapshot; memory survives compaction and is re-injected.
     */
    list(args?: NoArgs): Promise<ToolResult<MemoryListData>>;
    /**
     * Add or update small, durable session memory entries.
     */
    upsert(args: MemoryUpsertInput): Promise<ToolResult<MemoryUpsertData>>;
  };
}

declare const tools: Tools;
