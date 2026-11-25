import type {
  KnowledgeIssue,
  KnowledgeIssueArgs,
  KnowledgeIssueCode,
  KnowledgeRefState,
} from "./types";
import i18n from "../../i18n";

type Template = (args: KnowledgeIssueArgs) => string | null;

function str(args: KnowledgeIssueArgs, name: string): string | null {
  const value = args[name];
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return null;
}

/**
 * Display text per code, built from structured `args`. A template returns
 * null when an argument it needs is missing; the backend `message` is the fallback.
 */
const TEMPLATES: Record<KnowledgeIssueCode, Template> = {
  duplicate_key: (a) => {
    const key = str(a, "key");
    return key == null ? null : `Key "${key}" is duplicated.`;
  },
  empty_key: (a) => {
    const key = str(a, "key");
    return key ? `Key "${key}" is not valid.` : "Declaration is missing.";
  },
  filename_mismatch: (a) => {
    const key = str(a, "key");
    const file = str(a, "filename");
    return key == null || file == null
      ? null
      : `Filename "${file}" does not match declaration "${key}".`;
  },
  invalid_status: (a) => {
    const status = str(a, "status");
    return status == null
      ? null
      : `Status "${status}" is not valid. Use enabled, disabled, or pending.`;
  },
  self_relation: () => "Citation points at itself.",
  dangling_relation: (a) => {
    const path = str(a, "path");
    const reason = str(a, "reason");
    if (path && reason === "filename_mismatch") {
      return `Target "${path}" is not a valid knowledge node (filename does not match declaration).`;
    }
    if (path) {
      return `Target "${path}" is not a valid knowledge node.`;
    }
    const key = str(a, "key");
    return key == null ? null : `Citation "${key}" does not exist.`;
  },
  inactive_target: (a) => {
    const key = str(a, "key");
    const status = str(a, "target_status");
    return key == null || status == null ? null : `Cites ${status} "${key}".`;
  },
  missing_file: (a) => {
    const path = str(a, "path");
    return path == null ? null : `File "${path}" does not exist.`;
  },
  symlink_escape: (a) => {
    const path = str(a, "path");
    return path == null ? null : `"${path}" links outside the workspace.`;
  },
  invalid_lines: (a) => {
    const lines = str(a, "lines");
    if (lines == null) return null;
    return `Lines "${lines}" are not valid. Write #L12 or #L12-L30 (decimal, 1 <= A <= B).`;
  },
  unreadable_file: (a) => {
    const path = str(a, "path");
    const reason = str(a, "reason");
    return path == null || reason == null
      ? null
      : `"${path}" is not a readable text file (${reason}).`;
  },
  unsupported_language: (a) => {
    const path = str(a, "path");
    return path == null ? null : i18n.t("knowledge.symbolUnsupported", { path });
  },
  missing_symbol: (a) => {
    const path = str(a, "path");
    const symbol = str(a, "symbol");
    return path == null || symbol == null
      ? null
      : `Symbol "${symbol}" in "${path}" does not exist.`;
  },
  ambiguous_symbol: (a) => {
    const path = str(a, "path");
    const symbol = str(a, "symbol");
    return path == null || symbol == null
      ? null
      : `Symbol "${symbol}" in "${path}" is not unique.`;
  },
  line_out_of_range: (a) => {
    const path = str(a, "path");
    const lines = str(a, "lines");
    const count = str(a, "line_count");
    return path == null || lines == null || count == null
      ? null
      : `Lines ${lines} are outside "${path}", which has ${count} lines.`;
  },
};

export function isKnowledgeIssueCode(code: string): code is KnowledgeIssueCode {
  return Object.prototype.hasOwnProperty.call(TEMPLATES, code);
}

function knowledgeProblemText(
  code: string,
  args: KnowledgeIssueArgs | undefined,
  message: string,
): string {
  if (!isKnowledgeIssueCode(code)) return message;
  return TEMPLATES[code](args ?? {}) ?? message;
}

export function issueText(issue: KnowledgeIssue): string {
  return knowledgeProblemText(issue.code, issue.args, issue.message);
}

export function refStateText(ref: KnowledgeRefState): string {
  if (ref.status === "ok" || ref.status === "checking") return "";
  return knowledgeProblemText(ref.status, ref.args, ref.message);
}
