import { extractMentions, normalizeKey } from "./markers";
import type { KnowledgeNode } from "./types";

export type RefChipTone = "normal" | "disabled" | "warning" | "error";

export interface RefChipModel {
  key: string;
  label: string;
  targetId: string | null;
  tone: RefChipTone;
  jumpable: boolean;
}

/** Chip state for one mention. `id` is the lookup key; `label` is the visible text. */
export function chipForMarker(
  source: KnowledgeNode,
  marker: string,
  byKey: Map<string, KnowledgeNode>,
  label = marker,
): RefChipModel {
  const key = normalizeKey(marker);
  const target = byKey.get(key);
  if (!target) {
    return { key, label, targetId: null, tone: "error", jumpable: false };
  }
  if (normalizeKey(target.key) === normalizeKey(source.key)) {
    return { key, label, targetId: target.id, tone: "error", jumpable: false };
  }
  if (target.status === "disabled") {
    return { key, label, targetId: target.id, tone: "disabled", jumpable: true };
  }
  if (target.status === "pending") {
    return { key, label, targetId: target.id, tone: "warning", jumpable: true };
  }
  return { key, label, targetId: target.id, tone: "normal", jumpable: true };
}

/** One chip per body mention, labeled by the shortcode. */
export function relationStripChips(
  source: KnowledgeNode,
  byId: Map<string, KnowledgeNode>,
): RefChipModel[] {
  const byKey = new Map<string, KnowledgeNode>();
  for (const node of byId.values()) {
    const key = normalizeKey(node.key);
    if (key && !byKey.has(key)) byKey.set(key, node);
  }
  return extractMentions(source.value).map((mention) =>
    chipForMarker(source, mention.id, byKey, mention.label),
  );
}
