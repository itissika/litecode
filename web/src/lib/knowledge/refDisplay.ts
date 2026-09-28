import { extractMarkers, normalizeKey } from "./markers";
import type { KnowledgeNode } from "./types";

export type RefChipTone = "normal" | "disabled" | "warning" | "error";

export interface RefChipModel {
  key: string;
  targetId: number | null;
  tone: RefChipTone;
  jumpable: boolean;
}

export function bodyMarkerKeys(value: string): Set<string> {
  return new Set(extractMarkers(value).map((marker) => normalizeKey(marker)));
}

/** Chip state for a `[[key]]` marker in prose. */
export function chipForMarker(
  source: KnowledgeNode,
  marker: string,
  byKey: Map<string, KnowledgeNode>,
): RefChipModel {
  const key = normalizeKey(marker);
  const target = byKey.get(key);
  if (!target) {
    return { key, targetId: null, tone: "error", jumpable: false };
  }
  if (target.id === source.id) {
    return { key, targetId: target.id, tone: "error", jumpable: false };
  }
  if (!source.relations.includes(target.id)) {
    return { key, targetId: target.id, tone: "error", jumpable: false };
  }
  if (target.status === "disabled") {
    return { key, targetId: target.id, tone: "disabled", jumpable: true };
  }
  if (target.status === "pending") {
    return { key, targetId: target.id, tone: "warning", jumpable: true };
  }
  return { key, targetId: target.id, tone: "normal", jumpable: true };
}

/**
 * Relation-column chips shown only when the row exists and still has a
 * registration-side problem. Body markers own prose faults (unknown / unregistered).
 */
export function relationStripChips(
  source: KnowledgeNode,
  byId: Map<number, KnowledgeNode>,
  bodyKeys: Set<string>,
): RefChipModel[] {
  const chips: RefChipModel[] = [];
  const seen = new Set<number>();
  for (const rel of source.relations) {
    if (seen.has(rel)) continue;
    seen.add(rel);
    if (rel === source.id) continue;
    const target = byId.get(rel);
    if (!target) continue;
    const key = normalizeKey(target.key);
    const inBody = bodyKeys.has(key);
    const unused = !inBody;
    const inactive =
      source.status === "enabled" && target.status !== "enabled";
    if (unused) {
      chips.push({
        key,
        targetId: target.id,
        tone: "warning",
        jumpable: true,
      });
      continue;
    }
    if (!inactive || inBody) continue;
    const tone: RefChipTone =
      target.status === "disabled" ? "disabled" : "warning";
    chips.push({
      key,
      targetId: target.id,
      tone,
      jumpable: true,
    });
  }
  return chips;
}
