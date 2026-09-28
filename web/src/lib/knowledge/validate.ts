import { isKnowledgeKey, extractMarkers, normalizeKey } from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";

function statusLabel(status: KnowledgeNode["status"]): string {
  if (status === "disabled") return "已禁用";
  if (status === "pending") return "待审阅";
  return "启用";
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
    if (!key) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "empty_key",
        message: "声明缺失",
      });
    } else if (!isKnowledgeKey(key)) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "empty_key",
        message: `键「${key}」不合法`,
        ref: key,
      });
    } else if ((keyCounts.get(key) ?? 0) > 1) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "duplicate_key",
        message: `键「${key}」重复`,
        ref: key,
      });
    } else if (node.path && fileStem(node.path) !== key) {
      issues.push({
        nodeId: node.id,
        severity: "warning",
        code: "filename_mismatch",
        message: `文件名「${fileStem(node.path)}」与声明「${key}」不一致`,
        ref: key,
      });
    }

    const declared = new Set(node.relations.map((ref) => normalizeKey(ref)));
    for (const marker of new Set(extractMarkers(node.value))) {
      const markerKey = normalizeKey(marker);
      if (!declared.has(markerKey)) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "unregistered_marker",
          message: `正文标识「${markerKey}」未在引用声明中登记`,
          ref: markerKey,
        });
      }
    }

    const seen = new Set<string>();
    for (const ref of node.relations) {
      const refKey = normalizeKey(ref);
      if (!refKey || seen.has(refKey)) continue;
      seen.add(refKey);
      if (refKey === key) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "self_relation",
          message: "引用声明指向自己",
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
          message: `引用声明「${refKey}」不存在`,
          ref: refKey,
        });
        continue;
      }
      if (!extractMarkers(node.value).some((marker) => normalizeKey(marker) === refKey)) {
        issues.push({
          nodeId: node.id,
          severity: "warning",
          code: "unused_relation",
          message: `引用声明「${refKey}」未在正文使用`,
          ref: refKey,
        });
      }
      if (node.status === "enabled" && target.status !== "enabled") {
        issues.push({
          nodeId: node.id,
          severity: "warning",
          code: "inactive_target",
          message: `引用了${statusLabel(target.status)}的「${refKey}」`,
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
