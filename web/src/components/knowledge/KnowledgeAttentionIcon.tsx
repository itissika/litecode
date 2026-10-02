import { WarningCircle } from "@phosphor-icons/react";

import type { KnowledgeIssue } from "../../lib/knowledge/types";

/**
 * Amber exclamation after a knowledge title, for the warning-class issues the
 * rail badge counts. Only the icon carries them — the title text keeps its
 * error / pending / disabled tones untouched.
 */
export function KnowledgeAttentionIcon({
  warnings,
}: {
  warnings: KnowledgeIssue[];
}) {
  if (warnings.length === 0) return null;
  const message = warnings.map((issue) => issue.message).join("\n");
  return (
    <span
      className="knowledge-attention-icon"
      role="img"
      aria-label={`${warnings.length} 个问题需要关注`}
      title={message}
    >
      <WarningCircle size={12} weight="fill" aria-hidden />
    </span>
  );
}
