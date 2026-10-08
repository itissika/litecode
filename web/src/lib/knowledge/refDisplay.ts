import {
  extractMentions,
  humanFileLabel,
  humanSymbolLabel,
  isWorkspaceFileRef,
  normalizeKey,
} from "./markers";
import type { KnowledgeIssue, KnowledgeNode } from "./types";

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

/** A citation the chip can render. `line` is the start line, when the reference has one. */
export type Citation =
  | { kind: "node"; key: string }
  | { kind: "file"; path: string; symbol?: string | null; line?: number | null };

export type CitationTone = "normal" | "disabled" | "warning" | "drift" | "error";

export interface CitationChipModel {
  label: string;
  resolvable: boolean;
  tone: CitationTone;
  targetId: string | null;
  title?: string;
  file: boolean;
  symbol: boolean;
}

export interface CitationResolveInput {
  source: KnowledgeNode | null;
  /** Node looked up by key. Null when the key is not in the corpus. */
  target: KnowledgeNode | null;
  issues: readonly KnowledgeIssue[];
  /**
   * The editor already has a preview tab for this external path.
   * A path outside the workspace is resolvable only when this is true.
   */
  externalOpen: boolean;
  label?: string;
}

/**
 * One resolution for every chip.
 * A resolvable citation can be followed. Anything else stays a capsule so the
 * shortcode still round-trips, and the label does not navigate.
 */
export function resolveCitation(
  citation: Citation,
  input: CitationResolveInput,
): CitationChipModel {
  if (citation.kind === "node") return resolveNode(citation.key, input);
  return resolveFile(citation, input);
}

function resolveNode(marker: string, input: CitationResolveInput): CitationChipModel {
  const label = input.label?.trim() || normalizeKey(marker) || marker;
  const key = normalizeKey(marker);
  if (input.source) {
    const byKey = new Map<string, KnowledgeNode>();
    if (key && input.target) byKey.set(key, input.target);
    const chip = chipForMarker(input.source, marker, byKey, label);
    return {
      label: chip.label,
      resolvable: chip.jumpable && chip.targetId != null,
      tone: chip.tone === "error" ? "error" : chip.tone,
      targetId: chip.targetId,
      file: false,
      symbol: false,
    };
  }
  if (!input.target) {
    return {
      label,
      resolvable: false,
      tone: "error",
      targetId: null,
      file: false,
      symbol: false,
    };
  }
  const tone: CitationTone =
    input.target.status === "disabled"
      ? "disabled"
      : input.target.status === "pending"
        ? "warning"
        : "normal";
  return {
    label,
    resolvable: true,
    tone,
    targetId: input.target.id,
    file: false,
    symbol: false,
  };
}

function resolveFile(
  citation: Extract<Citation, { kind: "file" }>,
  input: CitationResolveInput,
): CitationChipModel {
  const path = citation.path;
  const symbol = citation.symbol?.trim() || "";
  const label =
    input.label?.trim() ||
    (symbol ? humanSymbolLabel(path, symbol) : humanFileLabel(path));
  const base = {
    label,
    targetId: null,
    file: true,
    symbol: symbol.length > 0,
  };
  const missingFile = input.issues.some(
    (issue) => issue.code === "missing_file" && issue.ref === path,
  );
  const missingSymbol =
    symbol.length > 0 &&
    input.issues.some((issue) => issue.code === "missing_symbol" && issue.ref === symbol);
  if (missingFile || missingSymbol) {
    return { ...base, resolvable: false, tone: "error" };
  }
  const drift = input.issues.find(
    (issue) => issue.code === "symbol_drift" && issue.ref === symbol,
  );
  // The editor can read a workspace path. An outside path is openable only
  // when a preview tab already holds the bytes.
  const resolvable = isWorkspaceFileRef(path) || input.externalOpen;
  if (!resolvable) return { ...base, resolvable: false, tone: "error" };
  if (drift) {
    return {
      ...base,
      resolvable: true,
      tone: "drift",
      title: drift.message || undefined,
    };
  }
  return { ...base, resolvable: true, tone: "normal" };
}
