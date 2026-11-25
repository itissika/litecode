import { X } from "@phosphor-icons/react";
import { useEffect, useMemo, type MouseEvent } from "react";

import { KNOWLEDGE_PRIVATE_ROOT, KNOWLEDGE_PUBLIC_ROOT } from "../lib/knowledge/load";
import { normalizeRefPath } from "../lib/knowledge/markers";
import { followCitation } from "../lib/knowledge/panel";
import {
  citationRefKey,
  citationSource,
  resolveCitation,
  type Citation,
  type CitationChipModel,
} from "../lib/knowledge/refDisplay";
import { externalPreviewId, useEditorStore } from "../stores/editorStore";
import {
  ensureKnowledgeLoaded,
  knowledgeRefState,
  lookupKnowledgeNodeByPath,
  useKnowledgeStore,
} from "../stores/knowledgeStore";
import { imageChipPath } from "./mention/imageChipHover";
import { ImageChipPreview } from "./mention/imageChipPreview";

function stopChipEvent(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
}

function chipClass(model: CitationChipModel, nodrag: boolean): string {
  const names = ["knowledge-token"];
  if (model.file) names.push("is-file");
  if (!model.resolvable) {
    names.push(model.file ? "is-missing" : "is-invalid");
  } else if (model.symbol) {
    names.push("is-symbol");
  }
  if (model.resolvable && model.tone === "drift") names.push("is-drift");
  if (model.resolvable && model.tone === "disabled") names.push("is-disabled");
  if (model.resolvable && model.tone === "warning") names.push("is-warning");
  if (nodrag) names.push("nodrag");
  return names.join(" ");
}

function isBarePathCitation(citation: Citation): boolean {
  return (
    !citation.symbol?.trim() &&
    citation.line == null &&
    !citation.lines?.trim() &&
    !citation.badLines?.trim()
  );
}

function looksLikeKnowledgePath(rawPath: string): boolean {
  const norm = normalizeRefPath(rawPath);
  return (
    norm.startsWith(`${KNOWLEDGE_PUBLIC_ROOT}/`) ||
    norm.startsWith(`${KNOWLEDGE_PRIVATE_ROOT}/`)
  );
}

/** The one citation capsule. A resolvable label follows the reference; remove is only for editors.
 *  Hover is the image preview layer: it opens immediately, and the full reference is its caption. */
export function CitationChip({
  citation,
  sourceId,
  label,
  onRemove,
  nodrag = false,
}: {
  citation: Citation;
  sourceId?: string;
  label?: string;
  onRemove?: () => void;
  nodrag?: boolean;
}) {
  const barePath = isBarePathCitation(citation);
  const maybeKnowledge = barePath && looksLikeKnowledgePath(citation.path);

  useEffect(() => {
    if (maybeKnowledge) ensureKnowledgeLoaded();
  }, [maybeKnowledge]);

  const source = useKnowledgeStore((state) =>
    sourceId ? state.byId.get(sourceId) : undefined,
  );
  const target = useKnowledgeStore((state) =>
    barePath ? lookupKnowledgeNodeByPath(state, citation.path) : undefined,
  );
  // Backend verdict for this citation, aligned by `ref_key` (KR-KEY-1).
  const refKey = useMemo(() => citationRefKey(citation), [citation]);
  const ref = useKnowledgeStore((state) => knowledgeRefState(state, sourceId, refKey));
  const ready = useKnowledgeStore((state) => state.ready);
  const externalOpen = useEditorStore((state) => {
    if (target) return false;
    const id = externalPreviewId(citation.path);
    return state.tabs.some((tab) => tab.external && tab.path === id);
  });
  const model = useMemo(
    () =>
      resolveCitation(citation, {
        source: source ?? null,
        target: target ?? null,
        ref: ref ?? null,
        ready,
        externalOpen,
        label,
      }),
    [citation, source, target, ref, ready, externalOpen, label],
  );

  // Copy hands the reference back as standard Markdown link text: the chip
  // itself is not selectable, so a selection that crosses it would otherwise
  // lose it (see `onCitationCopy`).
  const sourceLink = useMemo(
    () => citationSource(citation, target ?? null),
    [citation, target],
  );
  const detail = model.detail && model.detail !== model.label ? model.detail : null;
  // Full reference first, then the verdict text: a shortened label never hides
  // where it points, and an issue keeps its own line.
  const text = [detail, model.title].filter(Boolean).join("\n");
  const previewPath =
    model.file
      ? imageChipPath("file", {
          id: citation.path,
          symbol: citation.symbol,
          lines:
            citation.lines ??
            (citation.line != null && citation.line >= 1 ? String(citation.line) : null),
        })
      : null;

  return (
    <ImageChipPreview
      className={chipClass(model, nodrag)}
      path={previewPath}
      text={text}
      source={sourceLink}
    >
      {model.resolvable ? (
        <button
          type="button"
          className="knowledge-token-label nodrag"
          aria-label={model.label}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            followCitation(citation, sourceId);
          }}
        >
          {model.label}
        </button>
      ) : (
        <span className="knowledge-token-label">{model.label}</span>
      )}
      {onRemove ? (
        <button
          type="button"
          className="knowledge-token-remove nodrag"
          aria-label={`Remove ${model.label}`}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            onRemove();
          }}
        >
          <X size={10} weight="bold" />
        </button>
      ) : null}
    </ImageChipPreview>
  );
}
