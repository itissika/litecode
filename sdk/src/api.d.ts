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

type SymbolsData = {
  symbols: SymbolInfo[];
};

type LocationsData = {
  locations: Location[];
};

type BrowserPage = {
  active: boolean;
  agentOwned: boolean;
  id: string;
  title: string;
  url: string;
  visible: boolean;
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

type ExecCommandData = {
  /** Live session id; null once the session has exited. */
  session_id: string | null;
  /** Process exit code once the session has exited; null while it is live. */
  exit_code: Integer | null;
  /** Output since the previous call on this session; the first call returns output since spawn. */
  output: string;
  /** Workspace-relative path to the complete `.litecode/run/<job>.stdout` log (always full output on disk). */
  output_file: string;
};

type KnowledgeScope = {
  /** Restrict to these folders under the knowledge root. */
  folders?: string[];
  /** Restrict to these node keys. Union with folders when both are set. */
  nodes?: string[];
};

type KnowledgeNodeStatus = "enabled" | "disabled" | "pending" | "invalid";

type KnowledgeIssueRow = {
  key: string;
  folder: string;
  path: string;
  severity: "error" | "warning";
  code: string;
  ref_key: string | null;
  message: string;
};

interface TodoTools {
  /** Replace the current session task list; clearing occurs automatically when all items are completed. @tool todo.write @suite todo */
  write(args: {
    todos: {
      /** Task description. */
      content: string;
      /** Task status. */
      status: "pending" | "in_progress" | "completed";
    }[];
  }): Promise<{
    total: Integer;
    pending: Integer;
    in_progress: Integer;
    completed: Integer;
    cleared: boolean;
  }>;
}

interface PlanTools {
  /** Propose creating a workspace plan file (opens user approval dialog). Write the plan file, then present it and ask the user to confirm before executing. @tool plan.create @suite plan */
  create(args: {
    /** Short summary shown to the user for approval (not the full plan body). */
    summary: string;
  }): Promise<{
    approved: boolean;
    plan_path?: string;
    revision?: string;
    opinion?: string;
  }>;
  /** Clear the active plan pointer on this session when finished. @tool plan.finish @suite plan */
  finish(args?: NoArgs): Promise<{
    cleared: boolean;
  }>;
}

interface KnowledgeTools {
  /** List workspace knowledge nodes as flat summary rows (omit filters to list all); nodes use a `node` fence and `[@ key="..."]` / `[@ file="..."]` citations. @tool knowledge.list @suite knowledge */
  list(args?: KnowledgeScope): Promise<{
    root: string | null;
    total: Integer;
    nodes: {
      key: string;
      folder: string;
      path: string;
      status: KnowledgeNodeStatus;
      incoming_count: Integer;
      issue_count: Integer;
      summary: string;
    }[];
  }>;
  /** Check workspace knowledge nodes for broken citations, duplicate keys, or invalid node headers (omit filters to check all). @tool knowledge.check @suite knowledge */
  check(args?: KnowledgeScope): Promise<{
    root: string | null;
    checked: Integer;
    issue_count: Integer;
    issues: KnowledgeIssueRow[];
  }>;
  /** Inspect one knowledge node's markdown body, incoming/outgoing citations, and issues. @tool knowledge.refs @suite knowledge */
  refs(args: {
    key: string;
  }): Promise<{
    key: string;
    folder: string;
    path: string;
    status: KnowledgeNodeStatus;
    summary: string;
    body: string;
    incoming: {
      key: string;
      path: string;
      status: KnowledgeNodeStatus;
      summary: string;
    }[];
    outgoing: {
      key: string;
      path: string;
      status: KnowledgeNodeStatus;
      summary: string;
    }[];
    missing: string[];
    issues: KnowledgeIssueRow[];
  }>;
  /** Create a new pending knowledge node markdown stub under `.litecode/knowledge/`. @tool knowledge.create @suite knowledge */
  create(args: {
    key: string;
    folder?: string;
  }): Promise<{
    key: string;
    path: string;
    status: KnowledgeNodeStatus;
  }>;
}

interface WorkspaceTools {
  /** Read workspace setup or `.litecode/{excludes,mcp,custom_tools,apps}.json` configuration guides. @tool workspace.help @suite workspace */
  help(args?: {
    topic?: "overview" | "setup" | "excludes" | "mcp" | "custom_tools" | "apps";
  }): Promise<{
    topic: "overview" | "setup" | "excludes" | "mcp" | "custom_tools" | "apps";
    content: string;
  }>;
  /** Validate `.litecode/{excludes,mcp,custom_tools,apps}.json` in one check. @tool workspace.refresh @suite workspace */
  refresh(args?: NoArgs): Promise<{
    ok: boolean;
    summary: string;
    excludes_ok: boolean;
    mcp_ok: boolean;
    custom_tools_ok: boolean;
    apps_ok: boolean;
    issues: string[];
  }>;
  /** Read live runtime workspace status: active excludes, loaded MCP servers, custom tools, and Apps. @tool workspace.status @suite workspace */
  status(args?: NoArgs): Promise<{
    summary: string;
    excludes: string;
    mcp: {
      id: string;
      enabled: boolean;
      origin: string;
      state: string;
      command: string;
    }[];
    custom_tools: {
      id: string;
      suite: string | null;
      enabled: boolean;
      origin: string;
      timeout_secs: Integer;
      command: string;
    }[];
    apps: {
      id: string;
      kind: string;
      status: string;
      url: string | null;
      error: string | null;
    }[];
  }>;
}

interface SubagentTools {
  /** Launch a configured subagent child session asynchronously. @tool subagent.launch @suite subagent */
  launch(args: {
    /** Subagent id from allowed_subagents. */
    agent: string;
    /** Brief statement of the child's owned scope. */
    responsibility: string;
    /** Initial prompt delivered to the child session. */
    prompt: string;
  }): Promise<{
    child_session_id: string;
    turn_id: string;
    status: "running";
    responsibility: string;
  }>;
  /** Wait for selected running subagent turns (or snapshot active children) to settle. @tool subagent.wait @suite subagent */
  wait(args?: {
    /** Optional child_session_ids; omit to await/snapshot all currently running children. */
    ids?: string[];
    /** Number of selected turns to settle before returning. */
    count?: Integer;
  }): Promise<{
    status: "settled" | "nothing_to_wait_for";
    settled: {
      child_session_id: string;
      turn_id: string;
      reason: string;
      agent?: string;
      responsibility?: string;
      output: string;
      output_truncated?: boolean;
      cancelled_by?: string;
      transcript: string;
      start_line?: Integer;
      end_line?: Integer;
    }[];
    skipped: {
      id: string;
      reason: string;
    }[];
    summary: string;
  }>;
  /** Queue, steer, or stop-and-send a follow-up message to a child subagent session. @tool subagent.send @suite subagent */
  send(args: {
    id: string;
    message: string;
    mode?: "queue" | "steer" | "stop_and_send";
  }): Promise<{
    child_session_id: string;
    mode: string;
    status: "started" | "queued" | "steered" | "stopped_and_queued";
    turn_id?: string;
    position?: Integer;
    pending_id?: string;
    message: string;
  }>;
  /** Stop the active running turn of a child subagent session. @tool subagent.stop @suite subagent */
  stop(args: {
    id: string;
  }): Promise<{
    child_session_id: string;
    status: "stopped" | "nothing_running";
    turn_id?: string;
    reason?: string;
    output?: string;
    transcript?: string;
    message: string;
  }>;
  /** List all descendant child sessions of this session with current status and transcript metadata. @tool subagent.list @suite subagent */
  list(args?: NoArgs): Promise<{
    count: Integer;
    sessions: {
      id: string;
      agent: string;
      responsibility: string;
      state: string;
      transcript: string;
      transcript_lines?: Integer;
      step?: Integer;
      step_max?: Integer;
      reason?: string;
    }[];
  }>;
}

interface BotTools {
  /** Create a peer/owned bot with its own session in this workspace. @tool bot.create @suite bot */
  create(args: {
    name: string;
    description?: string;
  }): Promise<{
    id: string;
    name: string;
    session_id: string;
  }>;
  /** List visible bots in this org scope with live session status. @tool bot.list @suite bot */
  list(args?: NoArgs): Promise<{
    caller_id: string;
    bots: {
      id: string;
      name: string;
      session_id: string;
      status: string;
      is_self: boolean;
      description: string;
    }[];
  }>;
  /** Queue a message for another bot's session without waiting. @tool bot.send @suite bot */
  send(args: {
    bot: string;
    message: string;
  }): Promise<{
    id: string;
    name: string;
    session_id: string;
    queued: boolean;
  }>;
}

interface Tools {
  /** Read lines from one file or archived session transcript. @tool read @suite file */
  read(args: {
    path: Path; // Prefer a workspace-relative path. Past session transcripts are readable at .litecode/sessions/<session_id>.md.
    start_line?: Integer; // First line to read, 1-based, inclusive. @default 1
    limit?: Integer; // Lines to return. 1-5000. @default 2000
  }): Promise<{
    lines: Array<{
      line: Integer; // 1-based line number.
      text: string; // Line content.
    }>; // Lines for this page, in file order.
  }>;
  /** Replace a file's contents. @tool write @suite file */
  write(args: {
    content: string; // Content to write to the file.
    path: Path; // Prefer a workspace-relative path.
  }): Promise<{
    path: string; // Workspace-relative path inside the workspace; requested path outside it.
  }>;
  /** Apply replacements serially; matching edits stay saved on disk even if another item fails and rejects. A single edit is an array of one. @tool apply_edits @suite file */
  apply_edits(args: {
    edits: Array<{
      /** Prefer a workspace-relative path. */
      path: Path;
      /** Exact file text to find. Must match exactly one place. */
      old_string: string;
      /** Replacement text. Use an empty string to delete old_string. */
      new_string: string;
    }>; // Replacements applied serially; later items on the same path see earlier successful writes.
  }): Promise<{
    results: Array<{
      path: string;
    }>;
  }>;
  /** Find files by path glob. @tool glob @suite file */
  glob(args: {
    pattern: string; // Glob relative to path, or to the workspace root when path is omitted.
    limit?: Integer; // Maximum paths to return. 1-5000. @default 1000
    path?: Path; // Optional directory to search under.
  }): Promise<{
    paths: string[]; // Matching paths, sorted by depth, then parent directory, then name.
  }>;
  /** Search file contents. @tool grep @suite file */
  grep(args: {
    pattern: string; // Regular expression over file contents, unless literal is true.
    before?: Integer; // Lines of context before each hit. 0-10. @default 0
    after?: Integer; // Lines of context after each hit. 0-10. @default 0
    glob?: string; // One glob or comma-separated globs. A leading ! excludes. Relative to path.
    ignore_case?: boolean; // Ignore case when matching. @default false
    limit?: Integer; // Maximum hits to return. 1-100. @default 40
    literal?: boolean; // Search for pattern as plain text. @default false
    path?: Path; // File or directory to search. Defaults to the workspace.
  }): Promise<{
    hits: Array<{
      path: string; // Source file path.
      line: Integer; // 1-based match line.
      text: string; // Matching line text, capped at 500 characters.
    }>;
  }>;

  /** Semantic search over the workspace code index. @tool code_search @suite search */
  code_search(args: {
    query: string; // One natural-language or code-token query.
    include_pattern?: string; // Optional glob filter for workspace paths.
    min_score?: number; // Minimum raw dense cosine similarity for dense-only hits. 0–1. Lexical hits are not subject to this floor; 0 disables it. @default 0.8
    top_k?: Integer; // Maximum hits. 1–20. @default 8
  }): Promise<{
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
  }>;
  /** Search past session transcripts. @tool session_search @suite search */
  session_search(args: {
    query: string; // What to find in past session transcripts. Literal words or a phrase.
    session_id?: string; // Optional session scope: full id, unique prefix, or the handle from a result.
    own_session?: boolean; // Search only this session's transcript: the turns already folded out of the live context window. Cannot be combined with session_id.
    limit?: Integer; // Maximum rows. 1–50. @default 20
  }): Promise<{
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
  }>;

  /** Start a command. Waits up to yield_time_ms; a live session_id leaves it running in the background. Poll with write_stdin for output, or leave it and wait for the background-exit reminder (exit code + output_file). @tool exec_command @suite process */
  exec_command(args: {
    cmd: string; // Shell command to execute.
    workdir?: string; // Working directory; defaults to the turn cwd and must stay inside it.
    tty?: boolean; // True allocates a PTY; false or omitted uses plain pipes.
    yield_time_ms?: Integer; // Initial wait only; 250-600000 ms; default 600000. It does not limit the process lifetime.
    max_output_tokens?: Integer; // Output token budget for this call; defaults to 1 MiB in run_code or 10000 tokens otherwise.
  }): Promise<ExecCommandData>;
  /** Poll, feed, or interrupt an existing exec session. Empty chars waits until the session exits or yield_time_ms expires, returning accumulated output; on exit, exit_code is set and session_id is null. @tool write_stdin @suite process */
  write_stdin(args: {
    session_id: string; // Session id from exec_command.
    chars?: string; // Bytes to write; empty polls. "\u0003" sends SIGINT to the process group.
    yield_time_ms?: Integer; // Wait before yielding. Both modes accept 250-600000 ms; empty polls default 5000, non-empty writes default 250. Waiting inside a call pauses the run_code JS timer; one run_code call still has a 1800000 ms wall-clock cap.
    max_output_tokens?: Integer; // Output token budget for this call; defaults to 1 MiB in run_code or 10000 tokens otherwise.
  }): Promise<ExecCommandData>;

  /** Fetch one http(s) URL. @tool webfetch @suite web */
  webfetch(args: {
    url: string; // URL to fetch. Only http(s) URLs are accepted.
    format?: "text" | "markdown" | "html"; // Desired output format. @default "markdown"
  }): Promise<{
    body?: string; // Empty for a non-2xx answer: the status carries the fact.
    format?: "text" | "markdown" | "html";
    status?: Integer; // HTTP status of the final response, 2xx or not.
    url?: string;
  }>;
  /** Search the web. @tool websearch @suite web */
  websearch(args: {
    query: string; // Search query.
    top_k?: Integer; // Number of results, best first. 1–100. @default 10
  }): Promise<{
    // At most top_k hits, best first; empty when the search found nothing.
    results: {
      snippet: string; // Query-relevant excerpt from the page, as returned by the provider.
      title: string;
      url: string;
    }[];
  }>;

  /** Open a visible browser tab. @tool browser_open @suite browser */
  browser_open(args: {
    url: string; // http(s) URL to open in a visible browser tab.
  }): Promise<BrowserPage>;
  /** Close an agent-owned browser tab. @tool browser_close @suite browser */
  browser_close(args: {
    page_id: string; // Browser page id from browser_open or browser_list.
  }): Promise<{
    closed: boolean;
    id: string;
    fallback?: boolean;
  }>;
  /** List open browser tabs. @tool browser_list @suite browser */
  browser_list(args?: NoArgs): Promise<{
    active_id: string;
    pages: BrowserPage[];
  }>;
  /** Read the current visible browser page. @tool browser_read @suite browser */
  browser_read(args: {
    max_chars?: Integer; // Maximum body text characters. 1000–30000. @default 12000
    page_id?: string; // Optional page id. Omit for the active visible tab.
  }): Promise<{
    elements: {
      ref: string; // Element reference from the latest browser_read.
      tag: string;
      text: string;
      href?: string;
      type?: string;
    }[];
    page_id: string;
    text: string;
    title: string;
    truncated: boolean;
    url: string;
  }>;
  /** Click one interactive element on the visible page. @tool browser_click @suite browser */
  browser_click(args: {
    ref: string; // Element reference from the latest browser_read.
    page_id?: string; // Optional page id. Omit for the active visible tab.
  }): Promise<{
    page_id: string;
    ref: string;
    tag: string;
    text: string;
    title: string;
    url: string;
  }>;
  /** Type text into one input on the visible page. @tool browser_type @suite browser */
  browser_type(args: {
    ref: string; // Input reference from the latest browser_read.
    text: string; // Text to set on the focused input.
    page_id?: string; // Optional page id. Omit for the active visible tab.
  }): Promise<{
    page_id: string;
    ref: string;
    tag: string;
    text: string;
    title: string;
    url: string;
  }>;

  /** List diagnostics for one source file. @tool diagnostics @suite lsp */
  diagnostics(args: PathInput): Promise<{
    items: {
      severity: string; // Severity name mapped from the LSP number: Error, Warning, Information, Hint; Unknown for anything else.
      path: string; // Source file path.
      line: Integer; // 1-based line number.
      character: Integer; // 1-based UTF-16 column on that line.
      message: string; // Human-readable diagnostic.
    }[];
  }>;
  /** List symbols in one source file. @tool document_symbol @suite lsp */
  document_symbol(args: PathInput): Promise<SymbolsData>;
  /** Search symbols across the workspace. @tool workspace_symbol @suite lsp */
  workspace_symbol(args: {
    query: string; // Symbol name or fragment to search across the workspace.
    path?: string; // Optional source file that selects/starts the project's language server. Required only when more than one server is running.
  }): Promise<SymbolsData>;
  /** Fetch hover docs or type information at a file position. @tool hover @suite lsp */
  hover(args: RangeInput): Promise<
    // One hover answer. A hover is a single value, so there is no truncation.
    {
      markdown: string; // Flattened hover text, not raw LSP JSON.
    }
  >;
  /** List references at a file position. @tool find_references @suite lsp */
  find_references(args: RangeInput): Promise<LocationsData>;
  /** Jump to definition at a file position. @tool go_to_definition @suite lsp */
  go_to_definition(args: RangeInput): Promise<LocationsData>;
  /** Jump to implementations at a file position. @tool go_to_implementation @suite lsp */
  go_to_implementation(args: RangeInput): Promise<LocationsData>;

  /** Return the authoritative current session memory snapshot; memory survives compaction and is re-injected. @tool memory.list @suite memory */
  memory_list(args?: NoArgs): Promise<{
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
  }>;
  /** Add or update small, durable session memory entries. @tool memory.upsert @suite memory */
  memory_upsert(args: {
    // Memory entries to add or update.
    entries: {
      text: string; // Entry text, trimmed. 1-200 characters; duplicate text updates the existing entry.
      id?: string; // Optional existing id. Omit to add or update by text.
    }[];
  }): Promise<{
    added: string[]; // New ids.
    count: Integer; // Entries after the batch.
    rejected: RejectedItem[];
    total_chars: Integer; // Characters after the batch.
    updated: string[]; // Updated ids.
  }>;
  /** Delete entries by id from the current session's working memory. @tool memory.delete @suite memory */
  memory_delete(args: {
    ids: string[]; // Entry ids to delete.
  }): Promise<{
    count: Integer; // Entries after the batch.
    deleted: string[]; // Deleted ids.
    rejected: RejectedItem[];
  }>;
  /** Ask the user one or more structured questions via UI cards (free-text option included by system). @tool ask_user @suite ask_user */
  ask_user(args: {
    /** Optional card title shown above the questions. */
    summary?: string;
    questions: {
      /** Stable id keyed in answers. */
      id: string;
      /** Question text shown to the user. */
      prompt: string;
      /** Two or more choices for this question. */
      options: {
        /** Stable id returned in answers. */
        id: string;
        /** Display label shown to the user. */
        label: string;
      }[];
      /** When true, the user may select multiple choices. Default false. */
      multi_select?: boolean;
    }[];
  }): Promise<{
    outcome: "answered" | "skipped" | "no_foreground_session";
    answers: {
      id: string;
      selected: string[];
      free_text?: string;
    }[];
    text: string;
  }>;
  todo: TodoTools;
  plan: PlanTools;
  knowledge: KnowledgeTools;
  workspace: WorkspaceTools;
  subagent: SubagentTools;
  bot: BotTools;
}

declare const tools: Tools;
