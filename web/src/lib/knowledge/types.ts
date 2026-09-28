export type KnowledgeStatus = "enabled" | "disabled" | "pending";

/** Organizational container. Folders do not participate in citations. */
export interface KnowledgeFolder {
  id: number;
  name: string;
  parentId: number | null;
}

/** One knowledge node. Relations store ids; the body refers to other nodes by key. */
export interface KnowledgeNode {
  id: number;
  key: string;
  value: string;
  relations: number[];
  status: KnowledgeStatus;
  /** Folder that holds this node. `null` / omitted = root of the list and canvas. */
  folderId?: number | null;
}

export type KnowledgeIssueCode =
  | "duplicate_id"
  | "duplicate_key"
  | "empty_key"
  | "dangling_relation"
  | "self_relation"
  | "unknown_marker"
  | "unregistered_marker"
  | "unused_relation"
  | "inactive_target";

export type KnowledgeIssueSeverity = "error" | "warning";

export interface KnowledgeIssue {
  nodeId: number;
  severity: KnowledgeIssueSeverity;
  code: KnowledgeIssueCode;
  message: string;
  /** Key or id the problem is about, when there is one. */
  ref?: string;
}
