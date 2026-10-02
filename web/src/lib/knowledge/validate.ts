import {
  extractFileRefs,
  extractMarkers,
  isKnowledgeKey,
  isWorkspaceFileRef,
  normalizeKey,
} from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";

/** Paths already looked up. A missing key has not been checked yet. */
export type KnowledgeFilePresence = Readonly<Record<string, boolean>>;

function statusLabel(status: KnowledgeNode["status"]): string {
  if (status === "disabled") return "disabled";
  if (status === "pending") return "pending";
  return "enabled";
}

function fileStem(path: string): string {
  const base = path.split("/").pop() ?? path;
  return base.replace(/\.md$/i, "");
}

/** Check a knowledge set. Errors block a citation; warnings stay visible. */
export function validateKnowledge(nodes: KnowledgeNode[]): KnowledgeIssue[] {
  const issues: KnowledgeIssue[] = [];
  const keyCounts = new Map<string, number>();
  for (const node of nodes) {
    const key = normalizeKey(node.key);
    if (!isKnowledgeKey(key)) continue;
    keyCounts.set(key, (keyCounts.get(key) ?? 0) + 1);
  }

  const byKey = new Map<string, KnowledgeNode>();
  for (const node of nodes) {
    const key = normalizeKey(node.key);
    if (isKnowledgeKey(key) && !byKey.has(key)) byKey.set(key, node);
  }

  for (const node of nodes) {
    const key = normalizeKey(node.key);
    if (node.invalidStatus != null) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "invalid_status",
        message: `Status "${node.invalidStatus}" is not valid. Use enabled, disabled, or pending.`,
        ref: node.invalidStatus,
      });
    }
    if (!key) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "empty_key",
        message: "Declaration is missing.",
      });
    } else if (!isKnowledgeKey(key)) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "empty_key",
        message: `Key "${key}" is not valid.`,
        ref: key,
      });
    } else if ((keyCounts.get(key) ?? 0) > 1) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "duplicate_key",
        message: `Key "${key}" is duplicated.`,
        ref: key,
      });
    } else if (node.path && fileStem(node.path) !== key) {
      issues.push({
        nodeId: node.id,
        severity: "warning",
        code: "filename_mismatch",
        message: `Filename "${fileStem(node.path)}" does not match declaration "${key}".`,
        ref: key,
      });
    }

    const seen = new Set<string>();
    for (const refKey of extractMarkers(node.value)) {
      if (!refKey || seen.has(refKey)) continue;
      seen.add(refKey);
      if (refKey === key) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "self_relation",
          message: "Citation points at itself.",
          ref: refKey,
        });
        continue;
      }
      const target = byKey.get(refKey);
      if (!target) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "dangling_relation",
          message: `Citation "${refKey}" does not exist.`,
          ref: refKey,
        });
        continue;
      }
      if (node.status === "enabled" && target.status !== "enabled") {
        issues.push({
          nodeId: node.id,
          severity: "warning",
          code: "inactive_target",
          message: `Cites ${statusLabel(target.status)} "${refKey}".`,
          ref: refKey,
        });
      }
    }
  }

  return issues;
}

/**
 * File citations that are not in the workspace.
 * An illegal path (`..`, absolute) is an error immediately.
 * A legal path is an error only after a check has recorded it as absent.
 */
export function missingFileIssues(
  nodes: KnowledgeNode[],
  presence: KnowledgeFilePresence,
): KnowledgeIssue[] {
  const issues: KnowledgeIssue[] = [];
  for (const node of nodes) {
    const seen = new Set<string>();
    for (const path of extractFileRefs(node.value)) {
      if (seen.has(path)) continue;
      seen.add(path);
      const illegal = !isWorkspaceFileRef(path);
      if (!illegal && presence[path] !== false) continue;
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "missing_file",
        message: `File "${path}" does not exist.`,
        ref: path,
      });
    }
  }
  return issues;
}

export interface SymbolPresence {
  nodeId: string;
  file: string;
  symbol: string;
  exists: boolean;
  ambiguous: boolean;
  drifted: boolean;
}

/** Symbol citations whose chain is missing, repeated, or whose disk body differs from HEAD. */
export function symbolIssues(rows: SymbolPresence[]): KnowledgeIssue[] {
  const issues: KnowledgeIssue[] = [];
  for (const row of rows) {
    if (row.ambiguous || !row.exists) {
      issues.push({
        nodeId: row.nodeId,
        severity: "error",
        code: "missing_symbol",
        message: row.ambiguous
          ? `Symbol "${row.symbol}" in "${row.file}" is not unique.`
          : `Symbol "${row.symbol}" in "${row.file}" does not exist.`,
        ref: row.symbol,
      });
      continue;
    }
    if (!row.drifted) continue;
    issues.push({
      nodeId: row.nodeId,
      severity: "warning",
      code: "symbol_drift",
      message: `Symbol "${row.symbol}" in "${row.file}" differs from HEAD.`,
      ref: row.symbol,
    });
  }
  return issues;
}

export function groupIssues(
  issues: KnowledgeIssue[],
): Map<string, KnowledgeIssue[]> {
  const grouped = new Map<string, KnowledgeIssue[]>();
  for (const issue of issues) {
    const list = grouped.get(issue.nodeId);
    if (list) list.push(issue);
    else grouped.set(issue.nodeId, [issue]);
  }
  return grouped;
}

export function nodeHasError(
  issues: KnowledgeIssue[],
  nodeId: string,
): boolean {
  return issues.some(
    (issue) => issue.nodeId === nodeId && issue.severity === "error",
  );
}

/**
 * Rail badge for the knowledge icon. One number, the worse class only.
 * `inactive_target` stays in the list and off the badge.
 * `null` when nothing in either class is present.
 */
export function knowledgeRailBadge(
  issues: KnowledgeIssue[],
): { tone: "error" | "warning"; count: number } | null {
  const errors = new Set<string>();
  const warnings = new Set<string>();
  for (const issue of issues) {
    if (issue.code === "inactive_target") continue;
    if (issue.severity === "error") errors.add(issue.nodeId);
    else if (issue.severity === "warning") warnings.add(issue.nodeId);
  }
  if (errors.size > 0) return { tone: "error", count: errors.size };
  if (warnings.size > 0) return { tone: "warning", count: warnings.size };
  return null;
}

/** Side-list row: red = an error; amber = pending review. */
export function knowledgeListAlert(
  issues: KnowledgeIssue[],
  status: KnowledgeNode["status"],
): "red" | "amber" | null {
  if (issues.some((issue) => issue.severity === "error")) return "red";
  if (status === "pending") return "amber";
  return null;
}

/**
 * Title tone for a node in the side list and on the canvas — shared so both
 * stay in sync. A red fault reads as "unusable" and gets the same struck-through
 * treatment as a disabled node; disabled without a fault keeps the muted strike;
 * pending review turns the title amber.
 */
export function knowledgeTitleTone(
  issues: KnowledgeIssue[],
  status: KnowledgeNode["status"],
): "error" | "disabled" | "pending" | null {
  if (knowledgeListAlert(issues, status) === "red") return "error";
  if (status === "disabled") return "disabled";
  if (status === "pending") return "pending";
  return null;
}

/**
 * Warning-class issues that earn the amber exclamation beside a list-item or
 * canvas title. Same class the rail badge counts when no error is present:
 * `inactive_target` is left out because the relation edge already shows it.
 * Error-class issues stay on the title text tone and are not repeated here.
 */
export function knowledgeAttentionWarnings(
  issues: KnowledgeIssue[],
): KnowledgeIssue[] {
  return issues.filter(
    (issue) => issue.severity === "warning" && issue.code !== "inactive_target",
  );
}
