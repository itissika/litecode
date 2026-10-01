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

export type KnowledgeIssueCode =
  | "duplicate_key"
  | "empty_key"
  | "filename_mismatch"
  | "self_relation"
  | "dangling_relation"
  | "inactive_target"
  | "invalid_status"
  | "missing_file"
  | "missing_symbol"
  | "symbol_drift";

export type KnowledgeIssueSeverity = "error" | "warning";

export interface KnowledgeIssue {
  nodeId: string;
  severity: KnowledgeIssueSeverity;
  code: KnowledgeIssueCode;
  message: string;
  /** Key the problem is about, when there is one. */
  ref?: string;
}
