/** Whole JSON number. The host rejects fractional values. @asType integer */
type Integer = number;

/** Workspace-relative path, or an absolute path when explicitly allowed. */
type Path = string;

type LineText = {
  /** 1-based source line. */
  line: Integer;
  /** Exact line text. */
  text: string;
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

type ReadInput = {
  /** Prefer a workspace-relative path. Past session transcripts are readable at .litecode/sessions/<session_id>.md. */
  path: Path;
  /** First line to read, 1-based, inclusive. @default 1 @minimum 1 */
  start_line?: Integer;
  /** Maximum lines to return. @default 2000 @minimum 1 @maximum 5000 */
  limit?: Integer;
};

type ReadData = {
  ok: true;
  /** Line contents for this page, in file order. */
  lines: string[];
  message: string;
  path: string;
  start_line: Integer;
};

type WriteInput = {
  /** Content to write to the file. */
  content: string;
  /** Prefer a workspace-relative path. */
  path: Path;
  /** If true, refuse to overwrite an existing file. @default false */
  create_only?: boolean;
  /** sha256 of the current file bytes. Omit to skip the check. A mismatch resolves ok:false reason stale. */
  expected_hash?: string;
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

type WriteData = {
  ok: true;
  /** True when the file did not exist before. */
  created: boolean;
  /** File identity after the write. */
  file: FileRef;
  /** Post-write warnings such as LSP errors or path-risk notes. */
  message?: string;
};

type EditInput = {
  /** Replacement text. Use an empty string to delete old_string. */
  new_string: string;
  /** Exact file text to find. Must match exactly one place. */
  old_string: string;
  /** Prefer a workspace-relative path. */
  path: Path;
};

type EditFailureReason =
  | "not_found"
  | "no_match"
  | "invalid_args"
  | "empty"
  | "permission"
  | "io";

type EditData = {
  message: string;
  ok: boolean;
  path: string;
  reason?: EditFailureReason;
};

type ApplyEditsInput = {
  /** Replacements applied serially; later items on the same path see earlier successful writes. */
  edits: EditInput[];
};

type ApplyEditsData = {
  ok: true;
  /** Same length and order as edits. */
  results: EditData[];
};

type GlobInput = {
  /** Glob relative to path, or to the workspace root when path is omitted. */
  pattern: string;
  /** Maximum paths to return. @default 1000 @minimum 1 @maximum 5000 */
  limit?: Integer;
  /** Optional directory to search under. */
  path?: Path;
};

type GlobData = {
  ok: true;
  message: string;
  /** Matching paths sorted by depth, then parent directory, then name. */
  paths: string[];
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

type GrepHit = {
  /** Source file path. */
  path: string;
  /** 1-based match line. */
  line: Integer;
  /** Matching line text, capped at 500 characters. */
  text: string;
  /** Exact lines before the hit, when before > 0. */
  before?: LineText[];
  /** Exact lines after the hit, when after > 0. */
  after?: LineText[];
};

type GrepData = {
  ok: true;
  hits: GrepHit[];
  message: string;
  /** True when a limit or the 8K-token data guard removed results or context. */
  truncated?: boolean;
};

type CodeSearchStatus = "ok" | "off" | "warming" | "failed" | "busy";

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

type CodeSearchData = {
  ok: boolean;
  /** ok is true only for status ok; off/warming/failed/busy are soft domain failures. */
  status: CodeSearchStatus;
  hits: CodeSearchHit[];
  message?: string;
};

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

type WaitInput = {
  /** Command.id from a tools.run result whose exit_code is null. */
  id: string;
  /** Seconds to block waiting for exit. Values above 300 are clamped to 300. @minimum 1 */
  sec: Integer;
};

type KillInput = {
  /** Command.id from a tools.run or tools.wait result whose exit_code is null. */
  id: string;
};

type CommandData = {
  /** Process exit code when finished; null when still running or unavailable. */
  exit_code: Integer | null;
  /** While exit_code is null, use with tools.wait / tools.kill in this session. */
  id: string;
  /** Stdout plus truncation notes, spill path, or status line. Always present, may be empty. */
  output: string;
  /** Shell that ran the command. */
  shell: string;
  /** Stderr plus truncation notes. Empty when nothing was written. */
  stderr: string;
  /** True when wait observes a kill. */
  killed?: boolean;
  /** Warning about the command itself. */
  message?: string;
  /**
   * True when the tool call itself completed; this is not command success.
   * Check exit_code === 0 for process success. False on soft failures such as
   * an unreaped kill.
   */
  ok?: boolean;
  /** Present with ok:false; unreaped kill uses still_alive. */
  reason?: string;
};

/** run returns ok for a resolved call; exit_code carries the command status. */
type RunData = CommandData;
type WaitData = CommandData;
type KillData = CommandData;

type WebfetchFormat = "text" | "markdown" | "html";

type WebfetchInput = {
  /** URL to fetch. Only http(s) URLs are accepted. */
  url: string;
  /** Desired output format. @default "markdown" */
  format?: WebfetchFormat;
};

type WebfetchData = {
  message: string;
  ok: boolean;
  body?: string;
  format?: WebfetchFormat;
  status?: Integer;
  url?: string;
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

type WebsearchData = {
  message: string;
  ok: boolean;
  /** At most top_k hits, best first. */
  results: WebsearchHit[];
  query?: string;
};

type MemoryKind = "fact" | "preference" | "decision" | "constraint";

type MemoryEntry = {
  kind: MemoryKind;
  id: string;
  text: string;
  updated_at: Integer;
};

type RejectedItem = {
  /** 0-based input item index. */
  index: Integer;
  /** empty_text | text_too_long | invalid_kind | invalid_id | unknown_id | limit_exceeded. */
  reason: string;
  id?: string;
};

type MemoryListInput = {
  /** Optional kind filter. */
  kind?: MemoryKind;
  /** Optional case-insensitive substring over entry text. Max 100 characters. */
  query?: string;
};

type MemoryListData = {
  ok: true;
  /** Returned filtered entries. */
  count: Integer;
  /** Current entries, in insertion order. */
  entries: MemoryEntry[];
  /** Maximum entries the session can hold. */
  limit: Integer;
  /** Entries in the whole session memory. */
  total: Integer;
  /** Characters across all entry texts. */
  total_chars: Integer;
};

type MemoryUpsertEntry = {
  /** Optional kind; defaults to fact for new entries and keeps the current kind on update. */
  kind?: string;
  /** Entry text, trimmed. 1-200 characters; duplicate text updates the existing entry. */
  text: string;
  /** Optional existing id. Omit to add or update by text. */
  id?: string;
};

type MemoryUpsertInput = {
  /** Memory entries to add or update. */
  entries: MemoryUpsertEntry[];
};

type MemoryUpsertData = {
  ok: true;
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

type MemoryDeleteInput = {
  /** Entry ids to delete. */
  ids: string[];
};

type MemoryDeleteData = {
  ok: true;
  /** Entries after the batch. */
  count: Integer;
  /** Deleted ids. */
  deleted: string[];
  rejected: RejectedItem[];
};

type BrowserPage = {
  active: boolean;
  agentOwned: boolean;
  id: string;
  title: string;
  url: string;
  visible: boolean;
};

type BrowserElement = {
  /** Element reference from the latest browser_read. */
  ref: string;
  tag: string;
  text: string;
  href?: string;
  type?: string;
};

type BrowserOpenInput = {
  /** http(s) URL to open in a visible browser tab. */
  url: string;
};

type BrowserOpenData = BrowserPage & {
  ok: true;
};

type BrowserCloseInput = {
  /** Browser page id from browser_open or browser_list. */
  page_id: string;
};

type BrowserCloseData = {
  ok: true;
  closed: boolean;
  id: string;
  fallback?: boolean;
};

type BrowserListInput = {};

type BrowserListData = {
  ok: true;
  active_id: string;
  pages: BrowserPage[];
};

type BrowserReadInput = {
  /** Maximum body text characters. @default 12000 @minimum 1000 @maximum 30000 */
  max_chars?: Integer;
  /** Optional page id. Omit for the active visible tab. */
  page_id?: string;
};

type BrowserReadData = {
  ok: true;
  elements: BrowserElement[];
  page_id: string;
  text: string;
  title: string;
  truncated: boolean;
  url: string;
};

type BrowserClickInput = {
  /** Element reference from the latest browser_read. */
  ref: string;
  /** Optional page id. Omit for the active visible tab. */
  page_id?: string;
};

type BrowserClickData = {
  ok: true;
  page_id: string;
  ref: string;
  tag: string;
  text: string;
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

type BrowserTypeData = {
  ok: true;
  page_id: string;
  ref: string;
  tag: string;
  title: string;
  url: string;
};

type DiagnosticsInput = PathInput;
type DocumentSymbolInput = PathInput;
type FindReferencesInput = RangeInput;
type GoToDefinitionInput = RangeInput;
type GoToImplementationInput = RangeInput;
type HoverInput = RangeInput;

type WorkspaceSymbolInput = {
  /** Symbol name or fragment to search across the workspace. */
  query: string;
};

type DiagnosticsData = {
  ok: true;
  items: DiagnosticItem[];
  message: string;
};

type SymbolsData = {
  ok: true;
  message: string;
  symbols: SymbolInfo[];
};

type DocumentSymbolData = SymbolsData;
type WorkspaceSymbolData = SymbolsData;

type LocationsData = {
  ok: true;
  locations: Location[];
  message: string;
};

type FindReferencesData = LocationsData;
type GoToDefinitionData = LocationsData;
type GoToImplementationData = LocationsData;

type HoverData = {
  ok: true;
  /** Flattened hover text, not raw LSP JSON. */
  markdown: string;
  message: string;
};

interface MemoryTools {
  /** List the current session's working memory. @tool memory.list @suite memory @input MemoryListInput @output MemoryListData */
  list(args: MemoryListInput): Promise<ToolResult<MemoryListData>>;

  /** Add or update small, durable session memory entries. @tool memory.upsert @suite memory @input MemoryUpsertInput @output MemoryUpsertData */
  upsert(args: MemoryUpsertInput): Promise<ToolResult<MemoryUpsertData>>;

  /** Delete entries by id from the current session's working memory. @tool memory.delete @suite memory @input MemoryDeleteInput @output MemoryDeleteData */
  delete(args: MemoryDeleteInput): Promise<ToolResult<MemoryDeleteData>>;
}

interface Tools {
  /** Read lines from one file or archived session transcript. @tool read @suite file @input ReadInput @output ReadData */
  read(args: ReadInput): Promise<ToolResult<ReadData>>;

  /** Replace a file's contents; optional hash and create-only guards. @tool write @suite file @input WriteInput @output WriteData */
  write(args: WriteInput): Promise<ToolResult<WriteData>>;

  /** Apply one exact-string replacement. @tool edit @suite file @input EditInput @output EditData */
  edit(args: EditInput): Promise<ToolResult<EditData>>;

  /** Apply several edit replacements serially. @tool apply_edits @suite file @input ApplyEditsInput @output ApplyEditsData */
  apply_edits(args: ApplyEditsInput): Promise<ToolResult<ApplyEditsData>>;

  /** Find files by path glob. @tool glob @suite file @input GlobInput @output GlobData */
  glob(args: GlobInput): Promise<ToolResult<GlobData>>;

  /** Search file contents. @tool grep @suite file @input GrepInput @output GrepData */
  grep(args: GrepInput): Promise<ToolResult<GrepData>>;

  /** Semantic search over the workspace code index. @tool code_search @suite search @input CodeSearchInput @output CodeSearchData */
  code_search(args: CodeSearchInput): Promise<ToolResult<CodeSearchData>>;

  /** Start one shell command; use run_in_background for a non-blocking job. @tool run @suite process @input RunInput @output RunData */
  run(args: RunInput): Promise<ToolResult<RunData>>;

  /** Wait for a still-running command job; exit_code is the command status. @tool wait @suite process @input WaitInput @output WaitData */
  wait(args: WaitInput): Promise<ToolResult<WaitData>>;

  /** Terminate a command job in this session. @tool kill @suite process @input KillInput @output KillData */
  kill(args: KillInput): Promise<ToolResult<KillData>>;

  /** Fetch one http(s) URL. @tool webfetch @suite web @input WebfetchInput @output WebfetchData */
  webfetch(args: WebfetchInput): Promise<ToolResult<WebfetchData>>;

  /** Search the web. @tool websearch @suite web @input WebsearchInput @output WebsearchData */
  websearch(args: WebsearchInput): Promise<ToolResult<WebsearchData>>;

  /** Workspace memory namespace. */
  memory: MemoryTools;

  /** Open a visible browser tab. @tool browser_open @suite browser @input BrowserOpenInput @output BrowserOpenData */
  browser_open(args: BrowserOpenInput): Promise<ToolResult<BrowserOpenData>>;

  /** Close an agent-owned browser tab. @tool browser_close @suite browser @input BrowserCloseInput @output BrowserCloseData */
  browser_close(args: BrowserCloseInput): Promise<ToolResult<BrowserCloseData>>;

  /** List open browser tabs. @tool browser_list @suite browser @input BrowserListInput @output BrowserListData */
  browser_list(args: BrowserListInput): Promise<ToolResult<BrowserListData>>;

  /** Read the current visible browser page. @tool browser_read @suite browser @input BrowserReadInput @output BrowserReadData */
  browser_read(args: BrowserReadInput): Promise<ToolResult<BrowserReadData>>;

  /** Click one interactive element on the visible page. @tool browser_click @suite browser @input BrowserClickInput @output BrowserClickData */
  browser_click(args: BrowserClickInput): Promise<ToolResult<BrowserClickData>>;

  /** Type text into one input on the visible page. @tool browser_type @suite browser @input BrowserTypeInput @output BrowserTypeData */
  browser_type(args: BrowserTypeInput): Promise<ToolResult<BrowserTypeData>>;

  /** List diagnostics for one source file. @tool diagnostics @suite lsp @input DiagnosticsInput @output DiagnosticsData */
  diagnostics(args: DiagnosticsInput): Promise<ToolResult<DiagnosticsData>>;

  /** List symbols in one source file. @tool document_symbol @suite lsp @input DocumentSymbolInput @output DocumentSymbolData */
  document_symbol(args: DocumentSymbolInput): Promise<ToolResult<DocumentSymbolData>>;

  /** Search symbols across the workspace. @tool workspace_symbol @suite lsp @input WorkspaceSymbolInput @output WorkspaceSymbolData */
  workspace_symbol(args: WorkspaceSymbolInput): Promise<ToolResult<WorkspaceSymbolData>>;

  /** Fetch hover docs or type information at a file position. @tool hover @suite lsp @input HoverInput @output HoverData */
  hover(args: HoverInput): Promise<ToolResult<HoverData>>;

  /** List references at a file position. @tool find_references @suite lsp @input FindReferencesInput @output FindReferencesData */
  find_references(args: FindReferencesInput): Promise<ToolResult<FindReferencesData>>;

  /** Jump to definition at a file position. @tool go_to_definition @suite lsp @input GoToDefinitionInput @output GoToDefinitionData */
  go_to_definition(args: GoToDefinitionInput): Promise<ToolResult<GoToDefinitionData>>;

  /** Jump to implementations at a file position. @tool go_to_implementation @suite lsp @input GoToImplementationInput @output GoToImplementationData */
  go_to_implementation(args: GoToImplementationInput): Promise<ToolResult<GoToImplementationData>>;
}

declare const tools: Tools;