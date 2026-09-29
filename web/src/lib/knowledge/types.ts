export type KnowledgeStatus = "enabled" | "disabled" | "pending";

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
  /** Canvas origin relative to the parent folder. `null` = laid out on open. */
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
  | "invalid_status";

export type KnowledgeIssueSeverity = "error" | "warning";

export interface KnowledgeIssue {
  nodeId: string;
  severity: KnowledgeIssueSeverity;
  code: KnowledgeIssueCode;
  message: string;
  /** Key the problem is about, when there is one. */
  ref?: string;
}
