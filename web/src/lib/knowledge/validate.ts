import { extractMarkers, normalizeKey } from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";

function statusLabel(status: KnowledgeNode["status"]): string {
  if (status === "disabled") return "已禁用";
  if (status === "pending") return "待审阅";
  return "启用";
}

/** Check a knowledge set. Errors block a citation; warnings stay visible. */
export function validateKnowledge(nodes: KnowledgeNode[]): KnowledgeIssue[] {
  const issues: KnowledgeIssue[] = [];
  const idCounts = new Map<number, number>();
  const keyCounts = new Map<string, number>();
  for (const node of nodes) {
    idCounts.set(node.id, (idCounts.get(node.id) ?? 0) + 1);
    const key = normalizeKey(node.key);
    if (key) keyCounts.set(key, (keyCounts.get(key) ?? 0) + 1);
  }

  const byId = new Map<number, KnowledgeNode>();
  const byKey = new Map<string, KnowledgeNode>();
  for (const node of nodes) {
    if (!byId.has(node.id)) byId.set(node.id, node);
    const key = normalizeKey(node.key);
    if (key && !byKey.has(key)) byKey.set(key, node);
  }

  for (const node of nodes) {
    if ((idCounts.get(node.id) ?? 0) > 1) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "duplicate_id",
        message: `id ${node.id} 重复`,
      });
    }

    const key = normalizeKey(node.key);
    if (!key) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "empty_key",
        message: "键为空",
      });
    } else if ((keyCounts.get(key) ?? 0) > 1) {
      issues.push({
        nodeId: node.id,
        severity: "error",
        code: "duplicate_key",
        message: `键「${key}」重复`,
        ref: key,
      });
    }

    const markerIds = new Set<number>();
    for (const marker of new Set(extractMarkers(node.value))) {
      const target = byKey.get(normalizeKey(marker));
      if (!target) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "unknown_marker",
          message: `正文标识「${marker}」不存在`,
          ref: marker,
        });
        continue;
      }
      if (target.id === node.id) {
        markerIds.add(target.id);
        if (!node.relations.includes(node.id)) {
          issues.push({
            nodeId: node.id,
            severity: "error",
            code: "self_relation",
            message: "正文标识指向自己",
            ref: marker,
          });
        }
        continue;
      }
      markerIds.add(target.id);
      if (!node.relations.includes(target.id)) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "unregistered_marker",
          message: `正文标识「${marker}」未在关系列登记`,
          ref: marker,
        });
      }
    }

    const seen = new Set<number>();
    for (const rel of node.relations) {
      if (seen.has(rel)) continue;
      seen.add(rel);
      if (rel === node.id) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "self_relation",
          message: "关系指向自己",
        });
        continue;
      }
      const target = byId.get(rel);
      if (!target) {
        issues.push({
          nodeId: node.id,
          severity: "error",
          code: "dangling_relation",
          message: `关系指向不存在的 id ${rel}`,
          ref: String(rel),
        });
        continue;
      }
      if (!markerIds.has(rel)) {
        issues.push({
          nodeId: node.id,
          severity: "warning",
          code: "unused_relation",
          message: `关系「${normalizeKey(target.key)}」未在正文使用`,
          ref: normalizeKey(target.key),
        });
      }
      if (node.status === "enabled" && target.status !== "enabled") {
        issues.push({
          nodeId: node.id,
          severity: "warning",
          code: "inactive_target",
          message: `引用了${statusLabel(target.status)}的「${normalizeKey(target.key)}」`,
          ref: normalizeKey(target.key),
        });
      }
    }
  }

  return issues;
}

export function groupIssues(
  issues: KnowledgeIssue[],
): Map<number, KnowledgeIssue[]> {
  const grouped = new Map<number, KnowledgeIssue[]>();
  for (const issue of issues) {
    const list = grouped.get(issue.nodeId);
    if (list) list.push(issue);
    else grouped.set(issue.nodeId, [issue]);
  }
  return grouped;
}

export function nodeHasError(
  issues: KnowledgeIssue[],
  nodeId: number,
): boolean {
  return issues.some(
    (issue) => issue.nodeId === nodeId && issue.severity === "error",
  );
}
