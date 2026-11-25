import { refStateText } from "./issueText";
import {
  citationFact,
  citationMarkdown,
  humanFileLabel,
  humanRangeLabel,
  humanSymbolLabel,
  isWorkspaceFileRef,
  knowledgeRefKey,
  logicalKnowledgePath,
  normalizeKey,
  normalizeRefPath,
  normalizeSymbolQuery,
} from "./markers";
import type { KnowledgeNode, KnowledgeRefState, KnowledgeRefStatus } from "./types";

/**
 * Unified path-based citation rendered by `CitationChip`.
 * Whether it displays as a knowledge node chip or a file/symbol/range chip
 * is determined automatically from the path, `knowledgeStore`, and backend verdict.
 */
export interface Citation {
  path: string;
  symbol?: string | null;
  /** 1-based start line for jumping in the editor. */
  line?: number | null;
  /** Normalized line span (`12` or `12-30`), when valid. */
  lines?: string | null;
  /** Raw malformed line fragment, when invalid. */
  badLines?: string | null;
  /** Optional link label preserved for round-trip copy. */
  label?: string | null;
}

/** `ref_key` of one citation, matching Rust `Cite::ref_key` (KR-KEY-1). */
export function citationRefKey(citation: Citation): string {
  return knowledgeRefKey({
    path: citation.path,
    symbol: citation.symbol,
    lines: citation.symbol?.trim() ? null : rangeAttribute(citation) || null,
    badLines: citation.symbol?.trim() ? null : citation.badLines,
  });
}

/**
 * Standard CommonMark Markdown link representation for clipboard copy.
 * Pastes back into MentionEditor as the exact same chip and stays valid Markdown
 * anywhere else (GitHub, VS Code, Obsidian).
 */
export function citationSource(
  citation: Citation,
  targetNode?: Pick<KnowledgeNode, "key" | "path"> | null,
): string {
  const path = normalizeRefPath(citation.path);
  const symbol = citation.symbol ? normalizeSymbolQuery(citation.symbol) : "";
  const lines = symbol ? "" : rangeAttribute(citation);
  const badLines = symbol || lines ? "" : (citation.badLines?.trim() ?? "");
  const label =
    citation.label?.trim() ||
    (!symbol && !lines && !badLines && targetNode ? normalizeKey(targetNode.key) : "");
  return citationMarkdown({
    label: label || undefined,
    path: targetNode && !symbol && !lines && !badLines ? logicalKnowledgePath(targetNode.path) : path,
    symbol: symbol || null,
    lines: lines || null,
    badLines: badLines || null,
  });
}

type CitationTone = "normal" | "disabled" | "warning" | "drift" | "error";

const STATUS_TONE: Record<KnowledgeRefStatus, "ok" | "checking" | "error" | "drift"> = {
  ok: "ok",
  checking: "checking",
  inactive_target: "ok",
  dangling_relation: "error",
  self_relation: "error",
  missing_file: "error",
  symlink_escape: "error",
  invalid_lines: "error",
  unreadable_file: "error",
  unsupported_language: "error",
  missing_symbol: "error",
  ambiguous_symbol: "error",
  line_out_of_range: "error",
  duplicate_key: "error",
  empty_key: "error",
  invalid_status: "error",
  filename_mismatch: "error",
};

export function refStatusTone(
  ref: Pick<KnowledgeRefState, "status" | "severity">,
): "ok" | "checking" | "error" | "drift" {
  const known = (STATUS_TONE as Record<string, "ok" | "checking" | "error" | "drift">)[
    ref.status
  ];
  if (known) return known;
  if (ref.severity === "error") return "error";
  if (ref.severity === "warning") return "drift";
  return "ok";
}

export interface CitationChipModel {
  label: string;
  /** Full reference — path, range, or symbol — shown on chip hover. */
  detail?: string;
  resolvable: boolean;
  tone: CitationTone;
  targetId: string | null;
  title?: string;
  file: boolean;
  symbol: boolean;
}

interface CitationResolveInput {
  source: KnowledgeNode | null;
  /** Valid knowledge node matched by path (bare-path citations only). */
  target: KnowledgeNode | null;
  /** Backend verdict for this citation inside `source`, looked up by `ref_key`. */
  ref?: KnowledgeRefState | null;
  /** Snapshot `ready`. False: no citation shows an error yet. Default true. */
  ready?: boolean;
  /** The editor already has a preview tab for this external path. */
  externalOpen: boolean;
  label?: string;
}

/**
 * Single resolution for every chip.
 * Automatically folds bare-path citations pointing at a valid knowledge node (or
 * marked `kind === "node"` by the backend) into a knowledge node chip; everything
 * else renders as a file, symbol, or line-range chip.
 */
export function resolveCitation(
  citation: Citation,
  input: CitationResolveInput,
): CitationChipModel {
  const symbol = citation.symbol ? normalizeSymbolQuery(citation.symbol) : "";
  const range = symbol ? "" : rangeAttribute(citation);
  const bad = symbol || range ? "" : (citation.badLines?.trim() ?? "");
  const isBarePath = !symbol && !range && !bad;
  if (isBarePath && (input.target || input.ref?.kind === "node")) {
    return resolveNode(citation.path, input);
  }
  return resolveFile(citation, input);
}

function resolveNode(rawPath: string, input: CitationResolveInput): CitationChipModel {
  const normPath = normalizeRefPath(rawPath);
  const target = input.target;
  const ready = input.ready ?? true;
  const ref = ready ? (input.ref ?? null) : null;
  const defaultKey =
    (target ? normalizeKey(target.key) : "") ||
    (ref?.key ? normalizeKey(ref.key) : "");
  const rawLabel = input.label?.trim() ?? "";
  const isPathLikeLabel =
    rawLabel === normPath ||
    rawLabel === humanFileLabel(normPath) ||
    (target != null && rawLabel === logicalKnowledgePath(target.path));
  const label =
    (rawLabel && !isPathLikeLabel ? rawLabel : "") ||
    defaultKey ||
    humanFileLabel(normPath);
  const detail = target ? logicalKnowledgePath(target.path) : normPath;
  const base = {
    label,
    detail,
    targetId: target?.id ?? null,
    file: false,
    symbol: false,
  };

  if (ref && refStatusTone(ref) === "error") {
    return {
      ...base,
      resolvable: false,
      tone: "error",
      title: refStateText(ref) || undefined,
    };
  }
  if (!target) {
    if (!ready) return { ...base, resolvable: false, tone: "normal" };
    return {
      ...base,
      resolvable: false,
      tone: "error",
      title: ref ? refStateText(ref) || undefined : undefined,
    };
  }
  if (input.source && target.path === input.source.path) {
    if (ref && refStatusTone(ref) !== "error") {
      return { ...base, resolvable: true, tone: "normal" };
    }
    return {
      ...base,
      resolvable: false,
      tone: "error",
      title: ref ? refStateText(ref) || undefined : "Citation points at itself.",
    };
  }
  const tone: CitationTone =
    target.status === "disabled"
      ? "disabled"
      : target.status === "pending"
        ? "warning"
        : "normal";
  return {
    ...base,
    resolvable: true,
    tone,
  };
}

function resolveFile(citation: Citation, input: CitationResolveInput): CitationChipModel {
  const path = normalizeRefPath(citation.path);
  const symbol = citation.symbol ? normalizeSymbolQuery(citation.symbol) : "";
  const range = symbol ? "" : rangeAttribute(citation);
  const label = symbol
    ? humanSymbolLabel(path, symbol)
    : range
      ? humanRangeLabel(path, range)
      : humanFileLabel(path);
  const base = {
    label,
    detail: citationFact(path, symbol || null, range || null),
    targetId: null,
    file: true,
    symbol: symbol.length > 0,
  };
  const ready = input.ready ?? true;
  const ref = ready ? (input.ref ?? null) : null;
  const tone = ref ? refStatusTone(ref) : "checking";
  if (ref && tone === "error") {
    return { ...base, resolvable: false, tone: "error", title: refStateText(ref) || undefined };
  }
  const resolvable = isWorkspaceFileRef(path) || input.externalOpen;
  if (!resolvable) return { ...base, resolvable: false, tone: "error" };
  if (ref && tone === "drift") {
    return {
      ...base,
      resolvable: true,
      tone: "drift",
      title: refStateText(ref) || undefined,
    };
  }
  return { ...base, resolvable: true, tone: "normal" };
}

function rangeAttribute(citation: {
  line?: number | null;
  lines?: string | null;
}): string {
  const raw = citation.lines?.trim() ?? "";
  if (raw) return raw;
  return citation.line != null && citation.line >= 1 ? String(citation.line) : "";
}
