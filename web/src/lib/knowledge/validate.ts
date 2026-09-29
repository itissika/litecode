import { isKnowledgeKey, extractMarkers, normalizeKey } from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";

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
