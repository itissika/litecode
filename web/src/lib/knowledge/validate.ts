/**
 * Presentation over backend issues. The verdicts themselves come from the
 * snapshot (`GET /api/workspace/knowledge`); nothing here judges a citation.
 */
import type { KnowledgeIssue, KnowledgeNode } from "./types";

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
