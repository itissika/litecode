export type KnowledgeStatus = "enabled" | "disabled" | "pending";

/** A markdown file in the knowledge tree with no `node` declaration. */
export interface KnowledgeUnknownFile {
  /** Path relative to the knowledge root, forward slashes. */
  path: string;
  /** File name, including `.md`. */
  name: string;
  /** Folder that holds this file. `null` = the corpus root. */
  folderId: string | null;
}

/** Organizational container. Folders do not participate in citations. */
export interface KnowledgeFolder {
  /** Path relative to `.litecode/knowledge`, forward slashes. */
  id: string;
  name: string;
  parentId: string | null;
}

/** One knowledge node. `id` is the declared key when that key is unique. */
export interface KnowledgeNode {
  id: string;
  key: string;
  /** Markdown body, without the leading `node` declaration fence. */
  value: string;
  /** One-line summary from the declaration block. */
  summary: string;
  /** Keys of mentions in the body, first-seen order. */
  relations: string[];
  status: KnowledgeStatus;
  /** Raw `status` text when it is not enabled, disabled, or pending. */
  invalidStatus?: string | null;
  /** World coordinates on the canvas. `null` = laid out until dragged. */
  x: number | null;
  y: number | null;
  /** Expanded card size. `null` = fit the content. */
  w: number | null;
  h: number | null;
  /** Folder that holds this node. `null` = root of the list and canvas. */
  folderId?: string | null;
  /** Path of the source file relative to `.litecode/knowledge`. */
  path: string;
  /** Declaration lines this app does not own. Written back unchanged. */
  extras?: string[];
}

/**
 * Every problem code the backend snapshot can carry
 * (`docs/knowledge-references.md` §6). Declaration codes have no `refKey`.
 */
export type KnowledgeIssueCode =
  | "duplicate_key"
  | "empty_key"
  | "filename_mismatch"
  | "invalid_status"
  | "self_relation"
  | "dangling_relation"
  | "inactive_target"
  | "missing_file"
  | "symlink_escape"
  | "invalid_lines"
  | "unreadable_file"
  | "unsupported_language"
  | "missing_symbol"
  | "ambiguous_symbol"
  | "symbol_drift"
  | "line_out_of_range";

type KnowledgeIssueSeverity = "error" | "warning";

/** Structured issue arguments: `key`, `path`, `symbol`, `lines`, `target_status`, `reason`, `line_count`, ... */
export type KnowledgeIssueArgs = Readonly<Record<string, unknown>>;

export interface KnowledgeIssue {
  nodeId: string;
  severity: KnowledgeIssueSeverity;
  code: KnowledgeIssueCode;
  /** English fallback text from the backend. Display goes through `issueText`. */
  message: string;
  /** Key / path / chain the problem is about, when there is one (legacy display field). */
  ref?: string;
  /** The citation this issue belongs to. `null` for declaration issues. */
  refKey?: string | null;
  args?: KnowledgeIssueArgs;
}

type KnowledgeRefKind = "node" | "file" | "symbol" | "range";

/** `ok`, `checking`, or one of the citation issue codes. */
export type KnowledgeRefStatus = "ok" | "checking" | KnowledgeIssueCode;

/** Backend verdict for one citation of one node, aligned by `refKey`. */
export interface KnowledgeRefState {
  refKey: string;
  kind: KnowledgeRefKind;
  key: string | null;
  path: string | null;
  symbol: string | null;
  lines: string | null;
  raw: string;
  status: KnowledgeRefStatus;
  severity: KnowledgeIssueSeverity | null;
  /** Node refs: target node path (null when dangling). Others: the cited path. */
  targetPath: string | null;
  args: KnowledgeIssueArgs;
  message: string;
}
